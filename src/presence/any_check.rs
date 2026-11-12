//! The GW020 check: `Any` anywhere in a type expression. The type expressions are parameter and return
//! annotations, the annotation of an annotated assignment, the bound and default of a type parameter,
//! and the value of a `type` statement or of an assignment annotated `TypeAlias`. A name counts as
//! `typing.Any` only when an import binds it, so a class that happens to be called `Any` is not reported.

use std::collections::HashSet;
use std::path::Path;

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    Expr, ExprAttribute, ExprName, ExprStringLiteral, ExprSubscript, ModModule, Stmt,
    StmtAnnAssign, StmtFunctionDef, TypeParam, TypeParams,
};
use ruff_python_parser::parse_expression;
use ruff_source_file::SourceCode;
use ruff_text_size::{Ranged, TextRange};

use super::annotations::{ends_with_name, is_type_alias};
use super::{line_col, sequence_elements};
use crate::diagnostics::{Code, Diagnostic};
use crate::modules::{Import, ModuleScope};

const MESSAGE: &str = "'Any' is not allowed in type annotations";

/// The names in one module that refer to `typing.Any` and to the typing modules.
#[derive(Debug, Default)]
pub struct TypingNames {
    /// Names that an import binds to `typing.Any`, such as `Any` and `A` in `from typing import Any as A`.
    any: HashSet<String>,
    /// Names that an import binds to `typing` or `typing_extensions`, such as `t` in `import typing as t`.
    modules: HashSet<String>,
    /// Whether a bare `Any` is `typing.Any` because a module-level `from typing import *` or `from
    /// typing_extensions import *` provides it and no module-level import binds the name.
    star: bool,
}

impl TypingNames {
    /// Resolves the typing names of a module from its import tables.
    pub fn new(scope: &ModuleScope) -> Self {
        let star_import = ["typing", "typing_extensions"]
            .into_iter()
            .any(|module| scope.star_imports(module));
        let mut names = Self {
            star: star_import && scope.binding("Any").is_none(),
            ..Self::default()
        };
        for (name, import) in scope.bindings() {
            names.record(name, import);
        }
        names
    }

    /// Records `name` when its import binds a typing module, or `typing.Any` itself.
    fn record(&mut self, name: &str, import: &Import) {
        match import {
            Import::Module(module) if is_typing_module(module) => {
                self.modules.insert(name.to_owned());
            }
            Import::Member {
                module,
                name: member,
            } if is_typing_any(module, member) => {
                self.any.insert(name.to_owned());
            }
            _ => {}
        }
    }

    fn is_any(&self, name: &str) -> bool {
        self.any.contains(name) || (self.star && name == "Any")
    }

    fn is_module(&self, name: &str) -> bool {
        self.modules.contains(name)
    }
}

fn is_typing_module(module: &str) -> bool {
    matches!(module, "typing" | "typing_extensions")
}

/// Whether an import of `member` from `module` binds `typing.Any`.
fn is_typing_any(module: &str, member: &str) -> bool {
    is_typing_module(module) && member == "Any"
}

/// The spans of every `Any` in one type expression, in the order they are found. A string annotation is
/// parsed and walked, and an `Any` inside it is placed at the start of the outermost string literal.
pub fn any_occurrences(expr: &Expr, typing: &TypingNames) -> Vec<TextRange> {
    let mut finder = Finder {
        typing,
        found: Vec::new(),
    };
    finder.find(expr, None);
    finder.found
}

/// Collects the `Any` occurrences of one type expression.
struct Finder<'a> {
    typing: &'a TypingNames,
    found: Vec<TextRange>,
}

impl Finder<'_> {
    /// Walks a type expression. `anchor` is the string literal that the expression was parsed from, if
    /// any, and it places every occurrence found inside.
    fn find(&mut self, expr: &Expr, anchor: Option<TextRange>) {
        match expr {
            Expr::Name(name) => self.find_name(name, anchor),
            Expr::Attribute(attribute) => self.find_attribute(attribute, anchor),
            _ => self.find_other(expr, anchor),
        }
    }

    /// Walks a subscript or a string literal. Any other expression goes to the compound walk.
    fn find_other(&mut self, expr: &Expr, anchor: Option<TextRange>) {
        match expr {
            Expr::Subscript(subscript) => self.find_subscript(subscript, anchor),
            Expr::StringLiteral(literal) => self.find_string(literal, anchor),
            _ => self.find_compound(expr, anchor),
        }
    }

    /// Walks the parts of a compound type expression: the elements of a tuple or a list, the two
    /// sides of `|`, and the value of a starred expression. Any other expression has no `Any` in it
    /// that this walk can see.
    fn find_compound(&mut self, expr: &Expr, anchor: Option<TextRange>) {
        match expr {
            Expr::BinOp(binop) => {
                self.find(&binop.left, anchor);
                self.find(&binop.right, anchor);
            }
            Expr::Starred(starred) => self.find(&starred.value, anchor),
            _ => self.find_all(sequence_elements(expr), anchor),
        }
    }

    fn find_name(&mut self, name: &ExprName, anchor: Option<TextRange>) {
        if self.typing.is_any(name.id.as_str()) {
            self.hit(name.range(), anchor);
        }
    }

    /// `typing.Any` is an occurrence. Any other attribute is walked for its value only.
    fn find_attribute(&mut self, attribute: &ExprAttribute, anchor: Option<TextRange>) {
        if attribute.attr.as_str() == "Any" && self.is_typing_module(&attribute.value) {
            self.hit(attribute.range(), anchor);
        }
        self.find(&attribute.value, anchor);
    }

    fn hit(&mut self, range: TextRange, anchor: Option<TextRange>) {
        self.found.push(anchor.unwrap_or(range));
    }

    fn is_typing_module(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Name(name) if self.typing.is_module(name.id.as_str()))
    }

    fn find_all(&mut self, exprs: &[Expr], anchor: Option<TextRange>) {
        for expr in exprs {
            self.find(expr, anchor);
        }
    }

    /// The arguments of `Literal` are values, not types, so none are walked. Of `Annotated`, only the
    /// first argument is a type, and the rest is metadata.
    fn find_subscript(&mut self, subscript: &ExprSubscript, anchor: Option<TextRange>) {
        self.find(&subscript.value, anchor);
        if ends_with_name(&subscript.value, "Literal") {
            return;
        }
        let arguments: &Expr = if ends_with_name(&subscript.value, "Annotated") {
            annotated_type(&subscript.slice)
        } else {
            subscript.slice.as_ref()
        };
        self.find(arguments, anchor);
    }

    /// A string annotation stands for the expression its content parses to. Content that does not parse
    /// has no occurrences.
    fn find_string(&mut self, literal: &ExprStringLiteral, anchor: Option<TextRange>) {
        let Ok(parsed) = parse_expression(literal.value.to_str()) else {
            return;
        };
        let outer = Some(anchor.unwrap_or(literal.range()));
        self.find(parsed.syntax().body.as_ref(), outer);
    }
}

/// The type argument of an `Annotated` subscript: its first element when the slice is a tuple.
fn annotated_type(slice: &Expr) -> &Expr {
    match slice {
        Expr::Tuple(tuple) => tuple.elts.first().unwrap_or(slice),
        _ => slice,
    }
}

/// Reports every `Any` in a type expression of the module. The check is a walk of its own, so it does
/// not need the scopes of the presence visitor.
pub fn check_module(
    path: &Path,
    module: &ModModule,
    typing: &TypingNames,
    code: &SourceCode,
) -> Vec<Diagnostic> {
    let mut checker = AnyChecker {
        path,
        typing,
        code,
        diagnostics: Vec::new(),
    };
    checker.visit_body(&module.body);
    checker.diagnostics
}

struct AnyChecker<'a> {
    path: &'a Path,
    typing: &'a TypingNames,
    code: &'a SourceCode<'a, 'a>,
    diagnostics: Vec<Diagnostic>,
}

impl<'ast> Visitor<'ast> for AnyChecker<'_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::FunctionDef(function) => self.check_function(function),
            Stmt::ClassDef(class) => self.check_type_params(class.type_params.as_deref()),
            _ => self.check_statement(stmt),
        }
        visitor::walk_stmt(self, stmt);
    }
}

impl AnyChecker<'_> {
    /// The type expressions of a `def`: the bounds and defaults of its type parameters, its parameter
    /// annotations, and its return annotation.
    fn check_function(&mut self, function: &StmtFunctionDef) {
        self.check_type_params(function.type_params.as_deref());
        for parameter in function.parameters.iter() {
            if let Some(annotation) = parameter.annotation() {
                self.check_type(annotation);
            }
        }
        if let Some(returns) = &function.returns {
            self.check_type(returns);
        }
    }

    /// The type expressions of a statement other than a `def` or a `class`: a `type` statement, and an
    /// annotated assignment.
    fn check_statement(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::TypeAlias(alias) => {
                self.check_type_params(alias.type_params.as_deref());
                self.check_type(&alias.value);
            }
            Stmt::AnnAssign(assign) => self.check_annotated(assign),
            _ => {}
        }
    }

    /// The annotation is a type. The value is one only when the annotation is `TypeAlias`.
    fn check_annotated(&mut self, assign: &StmtAnnAssign) {
        self.check_type(&assign.annotation);
        let alias_value = assign
            .value
            .as_deref()
            .filter(|_| is_type_alias(&assign.annotation));
        if let Some(value) = alias_value {
            self.check_type(value);
        }
    }

    fn check_type_params(&mut self, type_params: Option<&TypeParams>) {
        let Some(type_params) = type_params else {
            return;
        };
        for expr in type_params.type_params.iter().flat_map(type_param_types) {
            self.check_type(expr);
        }
    }

    fn check_type(&mut self, expr: &Expr) {
        for range in any_occurrences(expr, self.typing) {
            let (line, col) = line_col(self.code, range.start());
            self.diagnostics.push(Diagnostic {
                path: self.path.to_path_buf(),
                line,
                col,
                code: Code::GW020,
                message: MESSAGE.to_owned(),
            });
        }
    }
}

/// The type expressions of one type parameter: its bound and its default.
fn type_param_types(type_param: &TypeParam) -> impl Iterator<Item = &Expr> {
    let (bound, default) = match type_param {
        TypeParam::TypeVar(param) => (param.bound.as_deref(), param.default.as_deref()),
        TypeParam::TypeVarTuple(param) => (None, param.default.as_deref()),
        TypeParam::ParamSpec(param) => (None, param.default.as_deref()),
    };
    bound.into_iter().chain(default)
}
