//! Walks a parsed module and reports the presence diagnostics.

use std::path::Path;
use std::rc::Rc;

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    Alias, AnyParameterRef, Decorator, ExceptHandler, Expr, ExprAttribute, ExprLambda, ExprName,
    Identifier, ModModule, Parameters, Pattern, Stmt, StmtAnnAssign, StmtAssign, StmtClassDef,
    StmtFunctionDef, StmtWith,
};
use ruff_source_file::SourceCode;
use ruff_text_size::TextSize;

use super::annotations::{FinalForm, ends_with_name, final_form, is_type_alias};
use super::class_index::ClassIndex;
use super::obvious;
use super::scopes::{
    Candidate, ClassScope, Closed, Escape, Receiver, ScopeKind, ScopeStack, Settled,
};
use super::{line_col, sequence_elements};
use crate::config::Mode;
use crate::diagnostics::{Code, Diagnostic};

/// Callees whose call declares a type, so `T = TypeVar("T")` is never reported. The attribute form,
/// such as `typing.TypeVar`, counts too.
const TYPE_DECLARATION_CALLEES: [&str; 6] = [
    "TypeVar",
    "ParamSpec",
    "TypeVarTuple",
    "NewType",
    "NamedTuple",
    "TypedDict",
];

pub struct ModuleJudgement {
    pub diagnostics: Vec<Diagnostic>,
    /// The class bodies that have exempt assignments, with the names those assignments declare.
    pub exempt: Vec<ExemptAttributes>,
}

pub struct ExemptAttributes {
    pub class_start: TextSize,
    pub names: Vec<String>,
}

/// Visits one module. The class index resolves the bases of its classes in the file at `path`, and the
/// obvious values of `mode` are judged with it too.
pub fn check_module(
    path: &Path,
    module: &ModModule,
    code: &SourceCode,
    index: &ClassIndex,
    mode: Mode,
) -> ModuleJudgement {
    let mut checker = Checker {
        path,
        index,
        code,
        mode,
        scopes: ScopeStack::default(),
        diagnostics: Vec::new(),
        exempt: Vec::new(),
        deferred_writes: Vec::new(),
    };
    checker.visit_body(&module.body);
    checker.close_scope();
    ModuleJudgement {
        diagnostics: checker.diagnostics,
        exempt: checker.exempt,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Report {
    /// The target binds its name, and needs no annotation of its own: a loop variable, a type declaration, a
    /// name inside an unpacking.
    Silent,
    Required,
    /// The value is obvious, so the first binding is exempt unless the name is bound again in its scope.
    Candidate,
}

/// An assignment through the receiver of a method, such as `self.x = 1`, with the class that judges it.
struct ReceiverWrite {
    class_start: TextSize,
    class: String,
    attr: String,
    offset: TextSize,
    undeclared: bool,
}

struct Checker<'a> {
    path: &'a Path,
    index: &'a ClassIndex,
    code: &'a SourceCode<'a, 'a>,
    mode: Mode,
    scopes: ScopeStack,
    diagnostics: Vec<Diagnostic>,
    exempt: Vec<ExemptAttributes>,
    /// The plain assignments through a receiver that an undeclared attribute judges. They wait for their class
    /// body to close, see `report_deferred_writes`.
    deferred_writes: Vec<ReceiverWrite>,
}

impl<'ast> Visitor<'ast> for Checker<'_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::FunctionDef(function) => self.visit_function(function),
            Stmt::ClassDef(class) => self.visit_class(class),
            _ => {
                self.bind_statement(stmt);
                visitor::walk_stmt(self, stmt);
            }
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Named(named) => {
                self.bind_target(&named.target, Report::Silent);
                visitor::walk_expr(self, expr);
            }
            Expr::Lambda(lambda) => self.visit_lambda(lambda),
            _ => visitor::walk_expr(self, expr),
        }
    }

    fn visit_except_handler(&mut self, except_handler: &'ast ExceptHandler) {
        let ExceptHandler::ExceptHandler(handler) = except_handler;
        if let Some(name) = &handler.name {
            self.scopes.bind(name.as_str(), name.range.start());
        }
        visitor::walk_except_handler(self, except_handler);
    }

    fn visit_pattern(&mut self, pattern: &'ast Pattern) {
        self.bind_capture(pattern);
        visitor::walk_pattern(self, pattern);
    }
}

impl<'ast> Checker<'_> {
    /// A `def` evaluates its decorators, defaults and annotations in the enclosing scope and binds
    /// its name there. Its body then runs in a new scope where the parameters are bound on entry.
    fn visit_function(&mut self, function: &'ast StmtFunctionDef) {
        self.check_function(function);
        self.visit_decorators(&function.decorator_list);
        self.visit_parameters(&function.parameters);
        if let Some(returns) = &function.returns {
            self.visit_annotation(returns);
        }
        self.scopes
            .bind(function.name.as_str(), function.name.range.start());
        let receiver = self.method_receiver(function);
        self.scopes.push(ScopeKind::Function { receiver });
        for parameter in function.parameters.iter() {
            let name = parameter.name();
            self.scopes.bind(name.as_str(), name.range.start());
        }
        self.visit_body(&function.body);
        self.close_scope();
    }

    /// A `class` evaluates its decorators and bases in the enclosing scope and binds its name there.
    /// Its body runs in a class scope that knows what the class declares.
    fn visit_class(&mut self, class: &'ast StmtClassDef) {
        self.visit_decorators(&class.decorator_list);
        if let Some(arguments) = &class.arguments {
            self.visit_arguments(arguments);
        }
        self.scopes
            .bind(class.name.as_str(), class.name.range.start());
        let facts = self.index.facts_for(self.path, class);
        self.scopes.push(ScopeKind::Class(ClassScope {
            name: class.name.as_str().to_owned(),
            start: class.range.start(),
            is_enum: facts.is_enum,
            declared: facts.declared.map(Rc::new),
        }));
        self.visit_body(&class.body);
        self.close_scope();
    }

    /// A lambda evaluates its parameter defaults in the enclosing scope. Its body runs in a function
    /// scope of its own, so a walrus in the body binds there and not in the enclosing scope.
    fn visit_lambda(&mut self, lambda: &'ast ExprLambda) {
        if let Some(parameters) = &lambda.parameters {
            self.visit_parameters(parameters);
        }
        self.scopes.push(ScopeKind::Function { receiver: None });
        self.visit_expr(&lambda.body);
        self.close_scope();
    }

    fn visit_decorators(&mut self, decorators: &'ast [Decorator]) {
        for decorator in decorators {
            self.visit_decorator(decorator);
        }
    }
}

impl Checker<'_> {
    fn check_function(&mut self, function: &StmtFunctionDef) {
        let in_class_body = matches!(self.scopes.current_kind(), ScopeKind::Class(_));
        self.check_parameters(function, in_class_body);
        if function.returns.is_none() && !init_is_exempt(function, in_class_body) {
            let message = format!(
                "function '{}' has no return type annotation",
                function.name.as_str()
            );
            self.report(function.name.range.start(), Code::GW003, message);
        }
    }

    fn check_parameters(&mut self, function: &StmtFunctionDef, in_class_body: bool) {
        for param in judged_parameters(function, in_class_body) {
            if param.annotation().is_none() {
                let message = format!(
                    "parameter '{}' of function '{}' has no type annotation",
                    param.name().as_str(),
                    function.name.as_str()
                );
                self.report(param.name().range.start(), Code::GW002, message);
            }
        }
    }

    /// A def directly in a class body is a method. A staticmethod has no receiver. A method whose class has an external
    /// ancestor has one too, but no attribute of it is judged.
    fn method_receiver(&self, function: &StmtFunctionDef) -> Option<Receiver> {
        let ScopeKind::Class(class) = self.scopes.current_kind() else {
            return None;
        };
        if is_staticmethod(function) {
            return None;
        }
        let first = function
            .parameters
            .posonlyargs
            .first()
            .or_else(|| function.parameters.args.first())?;
        Some(Receiver {
            name: first.parameter.name.as_str().to_owned(),
            class: class.name.clone(),
            start: class.start,
            declared: class.declared.clone(),
        })
    }

    fn bind_statement(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign(assign) => self.bind_assign(assign),
            Stmt::AnnAssign(assign) => self.bind_annotated(assign),
            Stmt::TypeAlias(alias) => self.bind_target(&alias.name, Report::Silent),
            Stmt::For(for_loop) => self.bind_target(&for_loop.target, Report::Silent),
            Stmt::With(with_stmt) => self.bind_with_targets(with_stmt),
            _ => self.bind_import_or_declaration(stmt),
        }
    }

    /// An annotated assignment through a receiver is reported at once, see `check_attribute_binding`.
    fn bind_annotated(&mut self, assign: &StmtAnnAssign) {
        self.check_annotated(assign);
        match assign.target.as_ref() {
            Expr::Attribute(attribute) => self.check_attribute_binding(attribute, true),
            target => self.bind_target(target, Report::Silent),
        }
    }

    fn bind_import_or_declaration(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Import(import) => self.bind_aliases(&import.names),
            Stmt::ImportFrom(import) => self.bind_aliases(&import.names),
            Stmt::Global(global) => self.declare_names(&global.names, Escape::Global),
            Stmt::Nonlocal(nonlocal) => self.declare_names(&nonlocal.names, Escape::Nonlocal),
            _ => {}
        }
    }

    fn bind_assign(&mut self, assign: &StmtAssign) {
        let report = self.assignment_report(assign);
        for target in &assign.targets {
            self.bind_target(target, report);
        }
    }

    fn assignment_report(&self, assign: &StmtAssign) -> Report {
        if is_type_declaration(assign) {
            Report::Silent
        } else if self.is_obvious_value(&assign.value) {
            Report::Candidate
        } else {
            Report::Required
        }
    }

    /// Whether `value` is obvious in the mode that is run. Only obvious mode exempts anything.
    fn is_obvious_value(&self, value: &Expr) -> bool {
        self.mode == Mode::Obvious && obvious::is_obvious(value, self.path, self.index)
    }

    fn bind_with_targets(&mut self, with_stmt: &StmtWith) {
        for target in with_stmt
            .items
            .iter()
            .filter_map(|item| item.optional_vars.as_deref())
        {
            self.bind_target(target, Report::Silent);
        }
    }

    fn bind_aliases(&mut self, aliases: &[Alias]) {
        for alias in aliases {
            self.scopes.bind(bound_name(alias), alias.range.start());
        }
    }

    /// `global` and `nonlocal` bind their names only inside a function. Elsewhere they bind nothing.
    fn declare_names(&mut self, names: &[Identifier], escape: Escape) {
        if matches!(self.scopes.current_kind(), ScopeKind::Function { .. }) {
            for name in names {
                self.scopes.declare(name.as_str(), escape);
            }
        }
    }

    fn bind_capture(&mut self, pattern: &Pattern) {
        if let Some(name) = captured_name(pattern) {
            self.scopes.bind(name.as_str(), name.range.start());
        }
    }

    /// Only a plain name can be judged, as `report` says. Names inside tuples, lists and starred targets are never
    /// judged. An attribute target is judged by the attribute rule, when it is bound through a method's receiver.
    fn bind_target(&mut self, target: &Expr, report: Report) {
        match target {
            Expr::Name(name) => self.bind_name(name, report),
            Expr::Attribute(attribute) => self.check_attribute_binding(attribute, false),
            _ => self.bind_nested_target(target),
        }
    }

    fn bind_nested_target(&mut self, target: &Expr) {
        match target {
            Expr::Starred(starred) => self.bind_target(&starred.value, Report::Silent),
            _ => self.bind_unpacked(sequence_elements(target)),
        }
    }

    fn bind_unpacked(&mut self, targets: &[Expr]) {
        for target in targets {
            self.bind_target(target, Report::Silent);
        }
    }

    fn bind_name(&mut self, name: &ExprName, report: Report) {
        let id = name.id.as_str();
        if report != Report::Silent && self.lacks_annotation(id) {
            self.judge_first_binding(name, report);
        }
        self.scopes.bind(id, name.range.start());
    }

    /// Judges the first binding of a plain name. An obvious value becomes a candidate, which is settled when its
    /// scope closes. Any other value is reported now. A constant at module level is never a candidate. A constant
    /// in a class body is one, and it is always reported when its scope closes, see `judge_settled`.
    fn judge_first_binding(&mut self, name: &ExprName, report: Report) {
        let id = name.id.as_str();
        if report == Report::Candidate && !self.is_module_constant(id) {
            self.scopes.add_candidate(id, name.range.start());
        } else {
            self.report_unannotated(name);
        }
    }

    fn is_module_constant(&self, name: &str) -> bool {
        matches!(self.scopes.current_kind(), ScopeKind::Module) && is_constant_name(name)
    }

    /// A plain name needs an annotation when it is not exempt, nothing in this scope has bound it yet,
    /// and the scope is not the body of an Enum class, whose plain assignments are members.
    fn lacks_annotation(&self, name: &str) -> bool {
        !is_exempt_name(name) && !self.scopes.is_bound(name) && !self.in_enum_body()
    }

    fn in_enum_body(&self) -> bool {
        matches!(self.scopes.current_kind(), ScopeKind::Class(class) if class.is_enum)
    }

    fn report_unannotated(&mut self, name: &ExprName) {
        let (code, message) = unannotated_message(self.scopes.current_kind(), name.id.as_str());
        self.report(name.range.start(), code, message);
    }

    /// Judges the candidates of a scope that has closed. A candidate that is bound again is reported, and
    /// the candidates of a class body that are not bound again declare their attributes.
    fn judge_closed(&mut self, closed: Closed) {
        let Closed { kind, settled } = closed;
        if let ScopeKind::Class(class) = &kind {
            self.report_deferred_writes(class, &settled);
        }
        let declared: Vec<String> = settled
            .into_iter()
            .filter_map(|item| self.judge_settled(&kind, item))
            .collect();
        self.declare_class_attributes(&kind, declared);
    }

    /// Reports the plain assignments that a method made through its receiver to an attribute the class does not
    /// declare, once the class body is closed. An obvious plain assignment of the same name in the class body is a
    /// candidate that is now bound again, and it is reported on its own line with the line of the assignment. So
    /// the assignment through the receiver is not reported again. Any other name is reported at its own line.
    fn report_deferred_writes(&mut self, class: &ClassScope, settled: &[Settled]) {
        let (own, rest): (Vec<ReceiverWrite>, Vec<ReceiverWrite>) =
            std::mem::take(&mut self.deferred_writes)
                .into_iter()
                .partition(|write| write.class_start == class.start);
        self.deferred_writes = rest;
        for write in own {
            let judged_in_body = settled.iter().any(|item| item.candidate.name == write.attr);
            if !judged_in_body {
                self.report_undeclared_write(&write);
            }
        }
    }

    /// Reports a candidate that is bound again. A class-body constant is reported even when it is not, because
    /// constants always need `Final[T]`. A candidate that is neither returns its name, because it is exempt.
    fn judge_settled(&mut self, kind: &ScopeKind, item: Settled) -> Option<String> {
        match item.rebound {
            Some(rebound) => {
                self.report_rebound(kind, &item.candidate, rebound);
                None
            }
            None if is_class_constant(kind, &item.candidate.name) => {
                let (code, message) = unannotated_message(kind, &item.candidate.name);
                self.report(item.candidate.offset, code, message);
                None
            }
            None => Some(item.candidate.name),
        }
    }

    fn declare_class_attributes(&mut self, kind: &ScopeKind, names: Vec<String>) {
        if let ScopeKind::Class(class) = kind
            && !names.is_empty()
        {
            self.exempt.push(ExemptAttributes {
                class_start: class.start,
                names,
            });
        }
    }

    /// The message gives the line of the first rebinding.
    fn report_rebound(&mut self, kind: &ScopeKind, candidate: &Candidate, rebound: TextSize) {
        let name = candidate.name.as_str();
        let (line, _) = line_col(self.code, rebound);
        let (code, message) = match kind {
            ScopeKind::Class(class) => (
                Code::GW004,
                format!(
                    "class attribute '{name}' of class '{}' has no type annotation (it is reassigned on line {line})",
                    class.name
                ),
            ),
            ScopeKind::Module | ScopeKind::Function { .. } => (
                Code::GW001,
                format!(
                    "variable '{name}' has no type annotation (it is reassigned on line {line})"
                ),
            ),
        };
        self.report(candidate.offset, code, message);
    }

    fn close_scope(&mut self) {
        let closed = self.scopes.close();
        self.judge_closed(closed);
    }

    /// GW004 for an attribute that a method binds through its receiver, such as `self.x = 1`, when the
    /// class does not declare `x`. The assignment is also noted for the class body, which needs to know
    /// whether a method assigns the name. An annotated assignment is reported at once. A plain one is reported
    /// when its class body closes, see `report_deferred_writes`. An augmented assignment never reaches this check.
    fn check_attribute_binding(&mut self, attribute: &ExprAttribute, annotated: bool) {
        let Some(write) = self.receiver_write(attribute) else {
            return;
        };
        self.scopes
            .note_receiver_assignment(&write.attr, write.offset);
        if !write.undeclared {
            return;
        }
        if annotated {
            self.report_undeclared_write(&write);
        } else {
            self.deferred_writes.push(write);
        }
    }

    /// `None` when `attribute` is not assigned through the receiver.
    fn receiver_write(&self, attribute: &ExprAttribute) -> Option<ReceiverWrite> {
        let ScopeKind::Function {
            receiver: Some(receiver),
        } = self.scopes.current_kind()
        else {
            return None;
        };
        if !is_name(&attribute.value, &receiver.name) {
            return None;
        }
        let attr = attribute.attr.as_str();
        Some(ReceiverWrite {
            class_start: receiver.start,
            class: receiver.class.clone(),
            attr: attr.to_owned(),
            offset: attribute.attr.range.start(),
            undeclared: receiver
                .declared
                .as_ref()
                .is_some_and(|declared| !declared.contains(attr)),
        })
    }

    fn report_undeclared_write(&mut self, write: &ReceiverWrite) {
        let message = format!(
            "attribute '{}' is not declared in the body of class '{}'",
            write.attr, write.class
        );
        self.report(write.offset, Code::GW004, message);
    }

    /// GW010 and GW011 for an annotated assignment. A bare `Final` is reported on a name or an attribute in any
    /// scope, unless its value is obvious. Other annotations are checked only for constant names.
    fn check_annotated(&mut self, assign: &StmtAnnAssign) {
        let form = final_form(&assign.annotation);
        if form == FinalForm::Bare && self.assigns_obvious_value(assign) {
            return;
        }
        self.check_annotated_target(assign.target.as_ref(), form, &assign.annotation);
    }

    fn check_annotated_target(&mut self, target: &Expr, form: FinalForm, annotation: &Expr) {
        match target {
            Expr::Name(name) => self.check_annotated_name(name, form, annotation),
            Expr::Attribute(attribute) if form == FinalForm::Bare => {
                self.report_bare_final(attribute.attr.range.start(), attribute.attr.as_str());
            }
            _ => {}
        }
    }

    fn assigns_obvious_value(&self, assign: &StmtAnnAssign) -> bool {
        assign
            .value
            .as_deref()
            .is_some_and(|value| self.is_obvious_value(value))
    }

    fn check_annotated_name(&mut self, name: &ExprName, form: FinalForm, annotation: &Expr) {
        match form {
            FinalForm::Bare => self.report_bare_final(name.range.start(), name.id.as_str()),
            FinalForm::NotFinal if self.needs_final(name, annotation) => {
                let id = name.id.as_str();
                let message = format!("constant '{id}' must be annotated as Final[T]");
                self.report(name.range.start(), Code::GW010, message);
            }
            FinalForm::NotFinal | FinalForm::Typed => {}
        }
    }

    /// Whether a constant of a module or class body is annotated without `Final`. A type alias is exempt.
    fn needs_final(&self, name: &ExprName, annotation: &Expr) -> bool {
        is_constant_scope(self.scopes.current_kind())
            && is_constant_name(name.id.as_str())
            && !is_type_alias(annotation)
    }

    fn report_bare_final(&mut self, offset: TextSize, name: &str) {
        let message = format!("'{name}' uses bare Final; write Final[T]");
        self.report(offset, Code::GW011, message);
    }

    fn report(&mut self, offset: TextSize, code: Code, message: String) {
        let (line, col) = line_col(self.code, offset);
        self.diagnostics.push(Diagnostic {
            path: self.path.to_path_buf(),
            line,
            col,
            code,
            message,
        });
    }
}

/// The parameters that GW002 judges. A method's first positional parameter (`self` or `cls`) is
/// exempt, unless the method is a staticmethod, where that parameter is an ordinary one.
fn judged_parameters(
    function: &StmtFunctionDef,
    in_class_body: bool,
) -> impl Iterator<Item = AnyParameterRef<'_>> {
    let exempt_first =
        in_class_body && !is_staticmethod(function) && has_positional(&function.parameters);
    function.parameters.iter().skip(usize::from(exempt_first))
}

/// A method named `__init__` needs no return annotation once one of its judged parameters is
/// annotated, so `def __init__(self, x: int)` passes while `def __init__(self)` does not.
fn init_is_exempt(function: &StmtFunctionDef, in_class_body: bool) -> bool {
    in_class_body
        && function.name.as_str() == "__init__"
        && judged_parameters(function, in_class_body).any(|param| param.annotation().is_some())
}

/// `ParametersIterator` yields `posonlyargs` and then `args` first, so its first item is the first
/// positional parameter exactly when one of those lists is not empty.
fn has_positional(parameters: &Parameters) -> bool {
    !parameters.posonlyargs.is_empty() || !parameters.args.is_empty()
}

fn is_staticmethod(function: &StmtFunctionDef) -> bool {
    function
        .decorator_list
        .iter()
        .any(|decorator| ends_with_name(&decorator.expression, "staticmethod"))
}

fn is_name(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Name(variable) if variable.id.as_str() == name)
}

/// The name an import binds: its alias, or else the first part of a dotted name, so `import a.b`
/// binds `a`. `from` imports never have dots in their names.
fn bound_name(alias: &Alias) -> &str {
    match &alias.asname {
        Some(asname) => asname.as_str(),
        None => alias
            .name
            .as_str()
            .split_once('.')
            .map_or(alias.name.as_str(), |(first, _)| first),
    }
}

/// The name that a pattern captures at its top level: `x` in `case x`, `*rest` in a sequence pattern, and
/// the rest of a mapping pattern.
fn captured_name(pattern: &Pattern) -> Option<&Identifier> {
    match pattern {
        Pattern::MatchAs(capture) => capture.name.as_ref(),
        Pattern::MatchStar(capture) => capture.name.as_ref(),
        _ => mapping_rest(pattern),
    }
}

fn mapping_rest(pattern: &Pattern) -> Option<&Identifier> {
    match pattern {
        Pattern::MatchMapping(mapping) => mapping.rest.as_ref(),
        _ => None,
    }
}

/// `_` and dunder names such as `__all__` are never reported.
fn is_exempt_name(name: &str) -> bool {
    name == "_" || is_dunder(name)
}

fn is_dunder(name: &str) -> bool {
    name.len() > 4 && name.starts_with("__") && name.ends_with("__")
}

/// A constant name is UPPER_CASE after any leading underscores, as in `^_*[A-Z][A-Z0-9_]*$`. A
/// dunder such as `__MAX__` is not a constant.
fn is_constant_name(name: &str) -> bool {
    let body = name.trim_start_matches('_');
    !is_dunder(name)
        && body.starts_with(|c: char| c.is_ascii_uppercase())
        && body.chars().all(is_constant_char)
}

fn is_constant_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'
}

/// Constants are judged at module and class level only. An UPPER_CASE name in a function is an
/// ordinary variable.
fn is_constant_scope(kind: &ScopeKind) -> bool {
    matches!(kind, ScopeKind::Module | ScopeKind::Class(_))
}

/// Whether a name is a constant of a class body. Such a constant is reported whether or not a method rebinds it.
fn is_class_constant(kind: &ScopeKind, name: &str) -> bool {
    matches!(kind, ScopeKind::Class(_)) && is_constant_name(name)
}

/// The code and message of a plain name that needs an annotation: GW004 in a class body, and GW001 in a module
/// or function scope.
fn unannotated_message(kind: &ScopeKind, name: &str) -> (Code, String) {
    match kind {
        ScopeKind::Class(class) => (
            Code::GW004,
            format!(
                "class attribute '{name}' of class '{}' has no type annotation",
                class.name
            ),
        ),
        ScopeKind::Module | ScopeKind::Function { .. } => (
            Code::GW001,
            format!("variable '{name}' has no type annotation"),
        ),
    }
}

fn is_type_declaration(assign: &StmtAssign) -> bool {
    matches!(assign.targets.as_slice(), [Expr::Name(_)]) && is_type_declaring_call(&assign.value)
}

fn is_type_declaring_call(value: &Expr) -> bool {
    let Expr::Call(call) = value else {
        return false;
    };
    TYPE_DECLARATION_CALLEES
        .iter()
        .any(|callee| ends_with_name(call.func.as_ref(), callee))
}
