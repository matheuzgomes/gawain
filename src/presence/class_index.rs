//! Class facts for the attribute rule: the attributes each class declares and the bases it has. The
//! module-level classes of every analysed file are indexed before any file is checked, so a class can
//! read the declarations of an ancestor defined in another file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Expr, Stmt, StmtClassDef};
use ruff_text_size::TextSize;

use super::LoadedFile;
use super::annotations::plain_name;
use crate::config::Mode;
use crate::modules::{Modules, Target};

/// One base class, as far as presence can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseRef {
    /// A class defined at module level of an analysed file.
    Project { file: PathBuf, class: String },
    /// `Enum` or a member of its family from the `enum` module.
    EnumFamily,
    /// `object`, `Generic` or `Protocol`. They add no attributes, so the ancestry is fully known.
    Neutral,
    /// Anything else: a class from another library, a builtin such as `Exception`, or a name that
    /// cannot be resolved. Its attributes are unknown.
    External,
}

impl BaseRef {
    fn is_external(&self) -> bool {
        matches!(self, BaseRef::External)
    }

    fn is_enum_family(&self) -> bool {
        matches!(self, BaseRef::EnumFamily)
    }
}

/// What a class body declares, and what its bases are.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassInfo {
    /// The names with an annotation in the class body, such as `x: int` or `x: Final = 1`, and the names that
    /// obvious mode exempts from an annotation.
    pub declared: HashSet<String>,
    pub bases: Vec<BaseRef>,
    /// The bases written without a subscript, such as `Box` in `class Sub(Box)`. A generic one makes the class
    /// generic too, see `ClassIndex::is_generic`.
    pub bare_bases: Vec<BaseRef>,
    /// Whether the class has type parameters of its own: PEP 695 parameters, a `Generic[...]` or `Protocol[...]`
    /// base, or a base subscripted with a type variable of its module, such as `Box[T]`. A base subscripted with
    /// concrete types, such as `Box[int]`, gives the class none.
    pub generic: bool,
}

/// What presence needs to know about one class, counting its ancestors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassFacts {
    /// Every attribute declared in the class or in an ancestor. `None` when an ancestor is external.
    pub declared: Option<HashSet<String>>,
    /// Whether an ancestor is an `Enum` class.
    pub is_enum: bool,
}

/// The head name and the attribute names of a dotted name, such as `("typing", ["Generic"])` for
/// `typing.Generic`.
type DottedName = (String, Vec<String>);

/// A class statement as its syntax gives it, before its bases are resolved. Its declarations are read from its
/// body, and each base is kept as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawClass {
    pub name: String,
    /// Where the class statement starts. It identifies the class in the analysed file.
    pub start: TextSize,
    declared: HashSet<String>,
    bases: Vec<RawBase>,
    /// Whether the class has PEP 695 type parameters, such as `class Bag[T]`.
    type_params: bool,
}

/// One base of a class statement, as its syntax gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawBase {
    /// The dotted name of the base, or `None` when it is not one, such as a call. A subscript stands for its value.
    name: Option<DottedName>,
    /// The dotted names that a subscript mentions, such as `T` in `Box[T]` or `tv.T` in `Box[tv.T]`. `None` when
    /// the base has no subscript.
    arguments: Option<Vec<DottedName>>,
}

impl RawBase {
    fn read(base: &Expr) -> Self {
        match base {
            Expr::Subscript(subscript) => Self {
                name: dotted_base(&subscript.value),
                arguments: Some(mentioned_names(&subscript.slice)),
            },
            _ => Self {
                name: dotted_base(base),
                arguments: None,
            },
        }
    }
}

impl RawClass {
    /// Reads a class statement: its name, the names that its body annotates, and its bases. Keyword arguments
    /// such as `metaclass=` are not bases, so they never reach this function.
    pub fn read(class: &StmtClassDef) -> Self {
        let bases: &[Expr] = match class.arguments.as_deref() {
            Some(arguments) => &arguments.args,
            None => &[],
        };
        Self {
            name: class.name.as_str().to_owned(),
            start: class.range.start(),
            declared: declared_names(&class.body),
            bases: bases.iter().map(RawBase::read).collect(),
            type_params: class.type_params.is_some(),
        }
    }

    /// Adds names that the class declares by its exempt assignments, which the class index then reads as
    /// declarations.
    pub fn declare(&mut self, names: impl IntoIterator<Item = String>) {
        self.declared.extend(names);
    }
}

/// The class statements of a module body, in order.
pub fn read_module_classes(body: &[Stmt]) -> Vec<RawClass> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class) => Some(RawClass::read(class)),
            _ => None,
        })
        .collect()
}

/// The module-level classes of every analysed file, and the modules their bases resolve in. Classes are
/// kept per file, so two files with one module name keep their own classes.
pub struct ClassIndex {
    modules: Modules,
    classes: HashMap<PathBuf, HashMap<String, ClassInfo>>,
}

/// Everything the walk up the ancestors learns about one class, and the state of the walk.
struct Lineage<'a> {
    index: &'a ClassIndex,
    declared: HashSet<String>,
    external: bool,
    enum_family: bool,
    /// Bases that the walk has not read yet.
    pending: Vec<&'a BaseRef>,
    /// Project classes that the walk has entered. A class is entered once, so a cycle ends the walk and a
    /// shared ancestor counts once.
    visited: HashSet<(&'a Path, &'a str)>,
}

impl<'a> Lineage<'a> {
    /// Takes in one base. An external base makes the attributes unknown, an enum base marks the family,
    /// and a project base brings in the class it names.
    fn absorb(&mut self, base: &'a BaseRef) {
        self.external |= base.is_external();
        self.enum_family |= base.is_enum_family();
        if let BaseRef::Project { file, class } = base {
            self.enter(file.as_path(), class.as_str());
        }
    }

    /// Enters the project class `class` of `module`. Its declarations and bases join the walk. A class no
    /// file defines makes the attributes unknown.
    fn enter(&mut self, file: &'a Path, class: &'a str) {
        if !self.visited.insert((file, class)) {
            return;
        }
        let ancestor = self.index.info(file, class);
        self.external |= ancestor.is_none();
        if let Some(ancestor) = ancestor {
            self.declared.extend(ancestor.declared.iter().cloned());
            self.pending.extend(ancestor.bases.iter());
        }
    }
}

impl ClassIndex {
    /// Indexes the module-level classes of `files`. Every file is registered before any base is classified,
    /// so a base can name a class from any file. When a name is defined twice in one file, the later
    /// definition is kept, because Python binds the last one. In obvious mode a name is also looked up through
    /// the star imports of its module, see `Modules::new`. Strict mode does not follow star imports.
    pub(super) fn build<'a>(files: impl IntoIterator<Item = &'a LoadedFile>, mode: Mode) -> Self {
        let files: Vec<&LoadedFile> = files.into_iter().collect();
        let mut modules = Modules::new(mode == Mode::Obvious);
        for file in &files {
            modules.insert(&file.path, &file.module_name, file.scope.clone());
        }
        let mut classes: HashMap<PathBuf, HashMap<String, ClassInfo>> = HashMap::new();
        for file in &files {
            let module_classes = classes.entry(file.path.clone()).or_default();
            for class in &file.classes {
                let info = class_info(&modules, &file.path, class);
                module_classes.insert(class.name.clone(), info);
            }
        }
        Self { modules, classes }
    }

    /// The facts of a class statement in the file at `file`, counting its ancestors. Its bases are resolved
    /// in that file. Each statement is judged on its own, so two module-level classes with one name keep
    /// their own facts.
    pub fn facts_for(&self, file: &Path, class: &StmtClassDef) -> ClassFacts {
        self.facts_of(&class_info(&self.modules, file, &RawClass::read(class)))
    }

    fn info(&self, file: &Path, class: &str) -> Option<&ClassInfo> {
        self.classes.get(file)?.get(class)
    }

    /// Whether `callee`, the function of a call in the file at `file`, names a generic class of the project. A name
    /// that is not a project class is not.
    pub fn is_generic_callee(&self, file: &Path, callee: &Expr) -> bool {
        let Some((head, rest)) = dotted_name(callee) else {
            return false;
        };
        match self.modules.resolve(file, head, &rest) {
            Target::Class { file, name } => self
                .info(&file, &name)
                .is_some_and(|info| self.is_generic(info, &mut HashSet::new())),
            _ => false,
        }
    }

    /// Whether the class `info` has type parameters that a call leaves open: its own, or those of a bare base
    /// that has them. A base subscripted with concrete types does not pass its parameters on. `visited` ends a
    /// cycle of bare bases.
    fn is_generic<'a>(
        &'a self,
        info: &'a ClassInfo,
        visited: &mut HashSet<(&'a Path, &'a str)>,
    ) -> bool {
        info.generic
            || info
                .bare_bases
                .iter()
                .any(|base| self.inherits_generic(base, visited))
    }

    /// Whether a bare base of a class is a project class that is generic, see `is_generic`.
    fn inherits_generic<'a>(
        &'a self,
        base: &'a BaseRef,
        visited: &mut HashSet<(&'a Path, &'a str)>,
    ) -> bool {
        match base {
            BaseRef::Project { file, class } => {
                visited.insert((file.as_path(), class.as_str()))
                    && self
                        .info(file, class)
                        .is_some_and(|ancestor| self.is_generic(ancestor, visited))
            }
            _ => false,
        }
    }

    fn facts_of(&self, info: &ClassInfo) -> ClassFacts {
        let lineage = self.lineage(info);
        ClassFacts {
            declared: (!lineage.external).then_some(lineage.declared),
            is_enum: lineage.enum_family,
        }
    }

    /// Walks the ancestors of `info`. Each base is read once, so the walk finishes on a cycle, and a
    /// diamond reads the shared ancestor once.
    fn lineage<'a>(&'a self, info: &'a ClassInfo) -> Lineage<'a> {
        let mut lineage = Lineage {
            index: self,
            declared: info.declared.clone(),
            external: false,
            enum_family: false,
            pending: info.bases.iter().collect(),
            visited: HashSet::new(),
        };
        while let Some(base) = lineage.pending.pop() {
            lineage.absorb(base);
        }
        lineage
    }
}

/// The declarations of a class and its bases, the bases resolved in `file`. A class is generic when it has
/// PEP 695 parameters, or when one of its bases is subscripted by a `Generic` or `Protocol`, or by a type
/// variable of the file, see `is_generic_base`.
fn class_info(modules: &Modules, file: &Path, class: &RawClass) -> ClassInfo {
    let classify = |base: &RawBase| classify_base(modules, file, base.name.as_ref());
    ClassInfo {
        declared: class.declared.clone(),
        bases: class.bases.iter().map(classify).collect(),
        bare_bases: class
            .bases
            .iter()
            .filter(|base| base.arguments.is_none())
            .map(classify)
            .collect(),
        generic: class.type_params
            || class
                .bases
                .iter()
                .any(|base| is_generic_base(modules, file, base)),
    }
}

/// Whether a base makes its class generic. `Generic[...]` and `Protocol[...]` do, and so does a subscript that
/// mentions a type variable, such as `Box[T]`, that the file defines or imports from a project module. A subscript
/// of concrete types, such as `Box[int]`, does not, and neither does a base without a subscript.
fn is_generic_base(modules: &Modules, file: &Path, base: &RawBase) -> bool {
    base.arguments.as_ref().is_some_and(|names| {
        classify_base(modules, file, base.name.as_ref()) == BaseRef::Neutral
            || names
                .iter()
                .any(|(head, rest)| is_type_var_reference(modules, file, head, rest))
    })
}

/// Whether the dotted name `head` with the attribute names `rest`, written in the file at `file`, is a type variable.
fn is_type_var_reference(modules: &Modules, file: &Path, head: &str, rest: &[String]) -> bool {
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    modules.is_type_var(file, head, &rest)
}

/// The names that the statements of a class body annotate, such as `x` in `x: int`.
fn declared_names(body: &[Stmt]) -> HashSet<String> {
    body.iter()
        .filter_map(declared_name)
        .map(str::to_owned)
        .collect()
}

/// The name that a statement declares by annotating it, when the target is a plain name.
fn declared_name(stmt: &Stmt) -> Option<&str> {
    match stmt {
        Stmt::AnnAssign(assign) => plain_name(&assign.target),
        _ => None,
    }
}

/// The dotted name of a base expression, or `None` when it is not one.
fn dotted_base(base: &Expr) -> Option<DottedName> {
    dotted_name(base).map(|(head, rest)| {
        (
            head.to_owned(),
            rest.into_iter().map(str::to_owned).collect(),
        )
    })
}

/// The dotted names that an expression mentions at any depth, such as `T` and `tv.U` in `dict[T, list[tv.U]]`.
fn mentioned_names(expr: &Expr) -> Vec<DottedName> {
    let mut collector = NameCollector::default();
    collector.visit_expr(expr);
    collector.names
}

#[derive(Default)]
struct NameCollector {
    names: Vec<DottedName>,
}

impl<'a> Visitor<'a> for NameCollector {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Some(name) = dotted_base(expr) {
            self.names.push(name);
        }
        visitor::walk_expr(self, expr);
    }
}

/// Classifies one base, given as its dotted name. A base that is not a dotted name is external.
fn classify_base(modules: &Modules, file: &Path, base: Option<&DottedName>) -> BaseRef {
    let Some((head, rest)) = base else {
        return BaseRef::External;
    };
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    classify_target(modules.resolve(file, head, &rest))
}

/// What a reference that the modules resolved to means for presence.
fn classify_target(target: Target) -> BaseRef {
    match target {
        Target::Class { file, name } => BaseRef::Project { file, class: name },
        Target::Member { module, name } => classify_member(&module, &name),
        other => classify_unresolved(&other),
    }
}

/// `object` is neutral. Every other reference that is not a project class or a known member is external.
fn classify_unresolved(target: &Target) -> BaseRef {
    match target {
        Target::Unbound(name) if name == "object" => BaseRef::Neutral,
        _ => BaseRef::External,
    }
}

/// The classes from `typing` and `enum` that presence knows by name.
fn classify_member(module: &str, name: &str) -> BaseRef {
    match (module, name) {
        ("typing" | "typing_extensions", "Generic" | "Protocol") => BaseRef::Neutral,
        ("enum", "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum") => {
            BaseRef::EnumFamily
        }
        _ => BaseRef::External,
    }
}

/// The head name and the attribute names of a dotted name, such as `("typing", ["Generic"])` for
/// `typing.Generic`.
fn dotted_name(expr: &Expr) -> Option<(&str, Vec<&str>)> {
    match expr {
        Expr::Name(name) => Some((name.id.as_str(), Vec::new())),
        Expr::Attribute(attribute) => dotted_name(&attribute.value).map(|(head, mut rest)| {
            rest.push(attribute.attr.as_str());
            (head, rest)
        }),
        _ => None,
    }
}
