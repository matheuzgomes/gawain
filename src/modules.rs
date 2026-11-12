//! Module names and import tables. A module-level name in an analysed file is resolved through that
//! file's imports, following re-exports, to the class or module it refers to.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use ruff_python_ast::statement_visitor::{self, StatementVisitor};
use ruff_python_ast::{Alias, Expr, Stmt, StmtImport, StmtImportFrom};

/// The file name of a package's initialiser.
const INIT_FILE: &str = "__init__.py";

/// Whether `path` names a package initialiser, which is the package it lives in.
pub fn is_package(path: &Path) -> bool {
    path.file_name().and_then(OsStr::to_str) == Some(INIT_FILE)
}

/// The dotted module name of the file at `path`. Its stem is the last part, and each parent directory
/// that holds an `__init__.py` adds its name in front. A file outside any package is named by its stem
/// alone, so `pkg/sub/mod.py` is `pkg.sub.mod` and `pkg/__init__.py` is `pkg`.
pub fn module_name(path: &Path) -> String {
    let resolved = location(path);
    let stem = resolved.file_stem().filter(|_| !is_package(&resolved));
    let mut parts: Vec<String> = stem
        .map(|stem| stem.to_string_lossy().into_owned())
        .into_iter()
        .collect();
    parts.extend(enclosing_packages(&resolved));
    parts.reverse();
    parts.join(".")
}

/// The path of the file with its directories resolved, so that `.`, `..` and symbolic links among the
/// directories give the file its place in the tree. The file's own name is kept: a symbolic link to a
/// file is named by the link, as Python names a module by the path it is imported through.
fn location(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    path.file_name()
        .map_or_else(|| path.to_path_buf(), |name| dir.join(name))
}

/// The names of the packages around the file at `path`, innermost first. A directory is a package while
/// it has a name and holds an `__init__.py`, and the walk ends at the first directory that is not one.
fn enclosing_packages(path: &Path) -> Vec<String> {
    path.parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .take_while(|dir| dir.file_name().is_some() && dir.join(INIT_FILE).is_file())
        .filter_map(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// What a name bound by an import refers to. Relative imports are already absolute here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// `import a.b` binds `a`, and `import a.b as k` binds `k`. Either way the name is a module.
    Module(String),
    /// `from m import n`, or with `as`: the name is the member `n` of module `m`.
    Member { module: String, name: String },
}

/// The names that one set of imports binds, and the modules that it star-imports, in source order.
#[derive(Debug, Default, Clone)]
pub struct ImportTable {
    bound: HashMap<String, Import>,
    star: Vec<String>,
}

/// The name that an alias of a plain `import` binds: `a` for `import a.b`, and `k` for `import a.b as k`.
fn import_bound_name(alias: &Alias) -> &str {
    match &alias.asname {
        Some(asname) => asname.as_str(),
        None => {
            let name = alias.name.as_str();
            name.split_once('.').map_or(name, |(first, _)| first)
        }
    }
}

/// The name that an alias of a `from` import binds: `n` for `from m import n`, and `k` for `n as k`.
fn from_bound_name(alias: &Alias) -> &str {
    alias
        .asname
        .as_ref()
        .map_or_else(|| alias.name.as_str(), |asname| asname.as_str())
}

impl ImportTable {
    fn add_import(&mut self, import: &StmtImport) {
        for alias in &import.names {
            let bound = import_bound_name(alias);
            let target = if alias.asname.is_some() {
                alias.name.as_str()
            } else {
                bound
            };
            self.bound
                .insert(bound.to_owned(), Import::Module(target.to_owned()));
        }
    }

    fn add_import_from(&mut self, module: &str, is_package: bool, import: &StmtImportFrom) {
        let from = import.module.as_ref().map(|name| name.as_str());
        let Some(base) = absolute_module(module, is_package, import.level, from) else {
            return;
        };
        for alias in &import.names {
            let name = alias.name.as_str();
            if name == "*" {
                self.star.push(base.clone());
                continue;
            }
            self.bound.insert(
                from_bound_name(alias).to_owned(),
                Import::Member {
                    module: base.clone(),
                    name: name.to_owned(),
                },
            );
        }
    }
}

/// The callees whose call makes a type variable, such as `TypeVar("T")` or `typing.ParamSpec("P")`.
const TYPE_VAR_CALLEES: [&str; 3] = ["TypeVar", "ParamSpec", "TypeVarTuple"];

/// The module-level names of one analysed module: the classes it defines and what its imports bind.
/// `imports` holds the imports at module level, which class bases resolve against. `anywhere` holds
/// every import in the module, at any depth, so the type-expression check sees imports inside a def,
/// a class body, an `if` or a `try`. `type_vars` holds the names that the module assigns a type variable,
/// a ParamSpec or a TypeVarTuple to.
#[derive(Debug, Default, Clone)]
pub struct ModuleScope {
    classes: HashSet<String>,
    imports: ImportTable,
    anywhere: ImportTable,
    type_vars: HashSet<String>,
}

impl ModuleScope {
    /// Reads the classes and imports in `body`, the statements of `module`. The last module-level binding
    /// of a name wins, so an import that comes after a class of the same name takes the name from it.
    pub fn new(module: &str, is_package: bool, body: &[Stmt]) -> Self {
        let mut scope = Self::default();
        for stmt in body {
            scope.bind_statement(module, is_package, stmt);
        }
        scope
            .type_vars
            .extend(body.iter().filter_map(type_var_name).map(str::to_owned));
        let mut walker = AnywhereImports {
            module,
            is_package,
            table: &mut scope.anywhere,
        };
        walker.visit_body(body);
        scope
    }

    /// Records what one module-level statement binds, in order. A class is recorded as a class, and an
    /// import as an import, which removes the class of the same name.
    fn bind_statement(&mut self, module: &str, is_package: bool, stmt: &Stmt) {
        match stmt {
            Stmt::ClassDef(class) => {
                self.classes.insert(class.name.as_str().to_owned());
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) => self.bind_import(module, is_package, stmt),
            _ => {}
        }
    }

    /// Binds the names of an import statement, which hide the classes of the same names.
    fn bind_import(&mut self, module: &str, is_package: bool, stmt: &Stmt) {
        if let Stmt::Import(import) = stmt {
            self.imports.add_import(import);
            self.rebind(import.names.iter().map(import_bound_name));
        }
        if let Stmt::ImportFrom(import) = stmt {
            self.imports.add_import_from(module, is_package, import);
            let aliases = import
                .names
                .iter()
                .filter(|alias| alias.name.as_str() != "*");
            self.rebind(aliases.map(from_bound_name));
        }
    }

    /// Forgets the classes whose names a later import binds at module level.
    fn rebind<'a>(&mut self, names: impl Iterator<Item = &'a str>) {
        for name in names {
            self.classes.remove(name);
        }
    }

    /// The import at module level that binds `name`, if any.
    pub fn binding(&self, name: &str) -> Option<&Import> {
        self.imports.bound.get(name)
    }

    /// Every name that an import binds, with the import. A module-level import hides a nested one that
    /// binds the same name.
    pub fn bindings(&self) -> impl Iterator<Item = (&str, &Import)> {
        let nested = self
            .anywhere
            .bound
            .iter()
            .filter(|(name, _)| !self.imports.bound.contains_key(*name));
        self.imports
            .bound
            .iter()
            .chain(nested)
            .map(|(name, import)| (name.as_str(), import))
    }

    /// Whether a statement of the module body star-imports `module`.
    pub fn star_imports(&self, module: &str) -> bool {
        self.imports.star.iter().any(|star| star == module)
    }
}

/// The name that a module-level statement binds to a type variable, such as `T` in `T = TypeVar("T")`.
fn type_var_name(stmt: &Stmt) -> Option<&str> {
    let Stmt::Assign(assign) = stmt else {
        return None;
    };
    match assign.targets.as_slice() {
        [Expr::Name(target)] if is_type_var_call(&assign.value) => Some(target.id.as_str()),
        _ => None,
    }
}

/// Whether `value` is a call of one of the type-variable callees, bare or through a module.
fn is_type_var_call(value: &Expr) -> bool {
    let Expr::Call(call) = value else {
        return false;
    };
    let callee = match call.func.as_ref() {
        Expr::Name(name) => name.id.as_str(),
        Expr::Attribute(attribute) => attribute.attr.as_str(),
        _ => return false,
    };
    TYPE_VAR_CALLEES.contains(&callee)
}

/// Walks every statement of a module and records each import it finds, at any depth.
struct AnywhereImports<'a> {
    module: &'a str,
    is_package: bool,
    table: &'a mut ImportTable,
}

impl<'ast> StatementVisitor<'ast> for AnywhereImports<'_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::Import(import) => self.table.add_import(import),
            Stmt::ImportFrom(import) => {
                self.table
                    .add_import_from(self.module, self.is_package, import);
            }
            _ => statement_visitor::walk_stmt(self, stmt),
        }
    }
}

/// The absolute name of the module that a `from` import reads. A relative import of level one
/// names the current package, and each further level goes one package up. `None` when the import
/// climbs above the top package, or when the module is not in a package at all.
fn absolute_module(
    module: &str,
    is_package: bool,
    level: u32,
    from: Option<&str>,
) -> Option<String> {
    if level == 0 {
        return from.map(str::to_owned);
    }
    let mut package = relative_package(module, is_package, level)?;
    if let Some(from) = from {
        package.push(from);
    }
    Some(package.join("."))
}

/// The package that a relative import of `level` dots, written in `module`, starts from. Level one is the
/// package of the module, and each further level goes one package up. `None` when that climbs above the
/// top package, or when no package is left.
fn relative_package(module: &str, is_package: bool, level: u32) -> Option<Vec<&str>> {
    let mut package: Vec<&str> = module.split('.').filter(|part| !part.is_empty()).collect();
    if !is_package {
        package.pop();
    }
    for _ in 1..level {
        package.pop()?;
    }
    (!package.is_empty()).then_some(package)
}

/// What a dotted reference resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A class defined at module level of an analysed file.
    Class { file: PathBuf, name: String },
    /// A module, analysed or only named by an import.
    Module(String),
    /// A member of a module that no analysed file defines as a class or a module, such as
    /// `typing.Generic`, or a name that is simply missing from the module.
    Member { module: String, name: String },
    /// A bare name that its module does not bind, such as the builtin `object`.
    Unbound(String),
    /// A type variable, a ParamSpec or a TypeVarTuple that its module assigns at module level, such as `T` in
    /// `T = TypeVar("T")`.
    TypeVar,
    /// A reference that cannot be followed: an attribute of a class, an import cycle, an import that
    /// climbs above the top package, or a module name that more than one analysed file has.
    Unknown,
}

/// The analysed files that a module name stands for.
enum Named<'a> {
    /// Exactly one analysed file has the name.
    Unique(&'a Path),
    /// Two or more analysed files have the name, so an import of it cannot be followed.
    Ambiguous,
    /// No analysed file has the name.
    Absent,
}

/// The module-level names of every analysed file, used to resolve references. A file is looked up by its
/// path, and a module name by the files that carry it. Two files outside any package can share a name, such
/// as `x/settings.py` and `y/settings.py`, so the name alone does not identify a file.
#[derive(Debug, Default)]
pub struct Modules {
    scopes: HashMap<PathBuf, ModuleScope>,
    files: HashMap<String, Vec<PathBuf>>,
    /// Whether a name that a module does not bind is looked up in the modules that it star-imports.
    follow_stars: bool,
}

impl Modules {
    /// An empty set of modules. Names are looked up through the star imports of a module when `follow_stars`
    /// is set, and are not otherwise.
    pub fn new(follow_stars: bool) -> Self {
        Self {
            follow_stars,
            ..Self::default()
        }
    }

    /// Registers the file at `path`, whose module name is `name`. Every file is kept, so a name that two
    /// files share is ambiguous.
    pub fn insert(&mut self, path: &Path, name: &str, scope: ModuleScope) {
        self.files
            .entry(name.to_owned())
            .or_default()
            .push(path.to_path_buf());
        self.scopes.insert(path.to_path_buf(), scope);
    }

    /// Resolves a dotted reference that is written in the file at `file`: its head name, such as `m` in
    /// `m.n.A`, and the attribute names after it, such as `["n", "A"]`. The head is looked up in that file
    /// first, so a class defined there is found even when another file has the same module name.
    pub fn resolve(&self, file: &Path, head: &str, rest: &[&str]) -> Target {
        rest.iter()
            .fold(self.head(file, head), |target, part| match target {
                Target::Module(path) => self.member(&path, part, &mut HashSet::new()),
                _ => Target::Unknown,
            })
    }

    /// Whether the dotted reference `head` with the attribute names `rest`, written in the file at `file`, names a
    /// type variable, a ParamSpec or a TypeVarTuple. It may be bound in that file, or imported from an analysed
    /// module, bare or through a module. A name that comes from a module that is not analysed is not one.
    pub fn is_type_var(&self, file: &Path, head: &str, rest: &[&str]) -> bool {
        self.resolve(file, head, rest) == Target::TypeVar
    }

    /// The name `name` as the file at `file` binds it at module level.
    fn head(&self, file: &Path, name: &str) -> Target {
        self.bound_target(file, name, &mut HashSet::new())
            .unwrap_or_else(|| Target::Unbound(name.to_owned()))
    }

    /// The member `name` of the module `module`, following re-exports. `visited` stops an import cycle.
    fn member(&self, module: &str, name: &str, visited: &mut HashSet<(String, String)>) -> Target {
        match self.named(module) {
            Named::Unique(file) => self.file_member(file, module, name, visited),
            Named::Ambiguous => Target::Unknown,
            Named::Absent => self.absent_member(module, name),
        }
    }

    /// The member `name` of the module `module`, which is the analysed file at `file`. Only this step can
    /// lead back to `member`, through an import, so only here is a visit recorded.
    fn file_member(
        &self,
        file: &Path,
        module: &str,
        name: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Target {
        if !visited.insert((module.to_owned(), name.to_owned())) {
            return Target::Unknown;
        }
        self.bound_target(file, name, visited)
            .unwrap_or_else(|| self.absent_member(module, name))
    }

    /// Which analysed files the module name `module` stands for.
    fn named(&self, module: &str) -> Named<'_> {
        match self.files.get(module).map(Vec::as_slice) {
            Some([file]) => Named::Unique(file.as_path()),
            Some(_) => Named::Ambiguous,
            None => Named::Absent,
        }
    }

    /// What the file at `file` binds `name` to, as a class defined there, through an import, or through a star
    /// import. `None` when it binds the name in none of these ways.
    fn bound_target(
        &self,
        file: &Path,
        name: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Option<Target> {
        let scope = self.scopes.get(file)?;
        self.local_target(scope, file, name, visited)
            .or_else(|| self.star_target(&scope.imports.star, name, visited))
    }

    /// What the module `scope`, which is the file at `file`, binds `name` to by its own classes and imports.
    fn local_target(
        &self,
        scope: &ModuleScope,
        file: &Path,
        name: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Option<Target> {
        if scope.classes.contains(name) {
            return Some(class_target(file, name));
        }
        if scope.type_vars.contains(name) {
            return Some(Target::TypeVar);
        }
        scope
            .imports
            .bound
            .get(name)
            .map(|import| self.import_target(import, visited))
    }

    /// What a name that the module binds nowhere itself refers to, through the star imports of the module.
    /// Python binds the names of the last star import first, so the imports are read from last to first, and
    /// the first one that provides the name wins. A name that starts with an underscore is never star-imported.
    fn star_target(
        &self,
        stars: &[String],
        name: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Option<Target> {
        if !self.follow_stars || name.starts_with('_') {
            return None;
        }
        stars
            .iter()
            .rev()
            .find_map(|module| self.star_member(module, name, visited))
    }

    /// The name that a star import of `module` provides, when `module` is an analysed file that binds it. A module
    /// that is not analysed, or whose name is shared by two files, provides nothing that gawain can see, so its
    /// star import is passed over as if it were absent. So is an answer that cannot be followed, such as one that
    /// loops back to the lookup in progress: `from . import sub` next to `from .mod import *` is a cycle that the
    /// package resolves to the submodule `sub`, not to an unknown.
    fn star_member(
        &self,
        module: &str,
        name: &str,
        visited: &mut HashSet<(String, String)>,
    ) -> Option<Target> {
        match self.named(module) {
            Named::Unique(file) if visited.insert((module.to_owned(), name.to_owned())) => self
                .bound_target(file, name, visited)
                .filter(|target| *target != Target::Unknown),
            _ => None,
        }
    }

    fn import_target(&self, import: &Import, visited: &mut HashSet<(String, String)>) -> Target {
        match import {
            Import::Module(path) => Target::Module(path.clone()),
            Import::Member {
                module: origin,
                name: origin_name,
            } => self.member(origin, origin_name, visited),
        }
    }

    /// A name that the module `module` does not bind: a submodule that is analysed, or else a plain member.
    fn absent_member(&self, module: &str, name: &str) -> Target {
        let submodule = format!("{module}.{name}");
        if self.files.contains_key(&submodule) {
            Target::Module(submodule)
        } else {
            Target::Member {
                module: module.to_owned(),
                name: name.to_owned(),
            }
        }
    }
}

fn class_target(file: &Path, name: &str) -> Target {
    Target::Class {
        file: file.to_path_buf(),
        name: name.to_owned(),
    }
}
