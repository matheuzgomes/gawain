//! Presence phase: reports names that lack the type annotations gawain requires.

mod annotations;
mod any_check;
mod class_index;
mod obvious;
mod scopes;
mod visitor;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::ThreadPoolBuildError;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use ruff_python_ast::token::TokenKind;
use ruff_python_ast::{Expr, ModModule, PySourceType};
use ruff_python_parser::{ParseError, Parsed, parse_unchecked_source};
use ruff_source_file::{LineIndex, SourceCode};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::config::Mode;
use crate::diagnostics::{Code, Diagnostic};
use crate::modules::{self, ModuleScope};
use crate::suppression;
use any_check::TypingNames;
use class_index::{ClassIndex, RawClass};
use visitor::ExemptAttributes;

/// Parses one module and returns its presence diagnostics, sorted as `check_files` sorts them. Only this file is
/// indexed, so bases that live in other files count as external. A file that cannot be parsed gives its GW000
/// errors.
pub fn check_source(path: &Path, source: &str, mode: Mode) -> Vec<Diagnostic> {
    let mut loaded = [LoadedFile::load(path, source.to_owned())];
    if mode == Mode::Obvious {
        declare_exempt_attributes(&mut loaded);
    }
    let index = ClassIndex::build(loaded.iter().filter_map(|file| file.as_ref().ok()), mode);
    let [file] = loaded;
    check_one(file, &index, mode)
}

/// Checks one loaded file against the index, or gives the GW000 diagnostics that it failed with, and sorts the
/// result. A diagnostic can be found after another that comes later in the file, as a candidate is judged when
/// its scope closes, so the sort puts them in source order.
fn check_one(
    file: Result<LoadedFile, Vec<Diagnostic>>,
    index: &ClassIndex,
    mode: Mode,
) -> Vec<Diagnostic> {
    let mut found = match file {
        Ok(file) => file.check(index, mode),
        Err(invalid) => invalid,
    };
    found.sort();
    found
}

/// The stack of every thread that parses or checks source, the main flow included. The parser and the
/// checks recurse once per level of nesting, and the drop of a tree does too, so a deeply nested file
/// needs far more than the default stack of a thread. The memory is reserved, not committed, until used.
pub const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Checks every file and returns the merged diagnostics, sorted.
///
/// A file that cannot be read, is not valid UTF-8, or does not parse gets its GW000 diagnostics instead. It is left
/// out of the class index, and no other rule runs on it, but every other file is still checked. `walk_errors` are
/// merged in unchanged.
///
/// The pool cannot be built when the operating system refuses a thread, and then no file is checked.
pub fn check_files(
    files: &[PathBuf],
    walk_errors: Vec<Diagnostic>,
    mode: Mode,
) -> Result<Vec<Diagnostic>, ThreadPoolBuildError> {
    let pool = ThreadPoolBuilder::new().stack_size(STACK_SIZE).build()?;
    Ok(pool.install(|| check_loaded(files, walk_errors, mode)))
}

/// The first pass keeps only what other files need of each file, so no syntax tree is alive for more than the work
/// on its own file. The last pass sorts each file's diagnostics on the thread that made them. Concatenating those
/// lists in file order gives the same order as one sort, because every diagnostic of a file has that file's path.
fn check_loaded(files: &[PathBuf], walk_errors: Vec<Diagnostic>, mode: Mode) -> Vec<Diagnostic> {
    let mut loaded: Vec<Result<LoadedFile, Vec<Diagnostic>>> = files
        .par_iter()
        .map(|path| LoadedFile::read(path))
        .collect();
    if mode == Mode::Obvious {
        declare_exempt_attributes(&mut loaded);
    }
    let index = ClassIndex::build(loaded.iter().filter_map(|file| file.as_ref().ok()), mode);
    let per_file: Vec<Vec<Diagnostic>> = loaded
        .into_par_iter()
        .map(|file| check_one(file, &index, mode))
        .collect();
    let mut diagnostics = walk_errors;
    diagnostics.reserve(per_file.iter().map(Vec::len).sum());
    diagnostics.extend(per_file.into_iter().flatten());
    diagnostics.sort();
    diagnostics
}

/// Obvious mode exempts some class-body assignments from GW004, and an exempt assignment declares its attribute
/// for the classes that inherit it. Whether an assignment is exempt depends on the class index, and the index
/// holds the declarations. So the index is built first, each file says which of its classes exempt names,
/// and the names are added before the index is built for good.
fn declare_exempt_attributes(loaded: &mut [Result<LoadedFile, Vec<Diagnostic>>]) {
    let index = ClassIndex::build(
        loaded.iter().filter_map(|file| file.as_ref().ok()),
        Mode::Obvious,
    );
    let exempt: Vec<Vec<ExemptAttributes>> = loaded
        .par_iter()
        .map(|file| {
            file.as_ref()
                .map_or_else(|_| Vec::new(), |file| file.exempt_attributes(&index))
        })
        .collect();
    for (file, exempt) in loaded.iter_mut().zip(exempt) {
        if let Ok(file) = file {
            file.declare_exempt(exempt);
        }
    }
}

/// What presence keeps of one file between the two passes. Its syntax tree is not kept, because the check parses the
/// source again.
struct LoadedFile {
    path: PathBuf,
    module_name: String,
    source: String,
    scope: ModuleScope,
    classes: Vec<RawClass>,
}

impl LoadedFile {
    fn read(path: &Path) -> Result<Self, Vec<Diagnostic>> {
        let bytes = std::fs::read(path).map_err(|error| {
            vec![Diagnostic::for_path(
                path,
                format!("cannot read file: {error}"),
            )]
        })?;
        let source = String::from_utf8(bytes).map_err(|_| {
            vec![Diagnostic::for_path(
                path,
                "file is not valid UTF-8".to_owned(),
            )]
        })?;
        Self::load(path, source)
    }

    /// The parser recovers from errors, so every error it reports is kept as a GW000.
    fn load(path: &Path, source: String) -> Result<Self, Vec<Diagnostic>> {
        let parsed = parse_unchecked_source(&source, PySourceType::Python);
        if !parsed.has_valid_syntax() {
            return Err(syntax_errors(path, &source, parsed.errors()));
        }
        let module_name = modules::module_name(path);
        let body = &parsed.syntax().body;
        let scope = ModuleScope::new(&module_name, modules::is_package(path), body);
        let classes = class_index::read_module_classes(body);
        Ok(Self {
            path: path.to_path_buf(),
            module_name,
            source,
            scope,
            classes,
        })
    }

    fn check(&self, index: &ClassIndex, mode: Mode) -> Vec<Diagnostic> {
        self.with_tree(|parsed, code| {
            let mut diagnostics =
                visitor::check_module(&self.path, parsed.syntax(), code, index, mode).diagnostics;
            let typing = TypingNames::new(&self.scope);
            diagnostics.extend(any_check::check_module(
                &self.path,
                parsed.syntax(),
                &typing,
                code,
            ));
            let suppressions =
                suppression::parse_suppressions(&self.path, comments(parsed, &self.source), code);
            suppression::apply(diagnostics, &suppressions)
        })
    }

    fn exempt_attributes(&self, index: &ClassIndex) -> Vec<ExemptAttributes> {
        self.with_tree(|parsed, code| {
            visitor::check_module(&self.path, parsed.syntax(), code, index, Mode::Obvious).exempt
        })
    }

    fn declare_exempt(&mut self, exempt: Vec<ExemptAttributes>) {
        let mut by_start: HashMap<TextSize, Vec<String>> = exempt
            .into_iter()
            .map(|found| (found.class_start, found.names))
            .collect();
        for class in &mut self.classes {
            if let Some(names) = by_start.remove(&class.start) {
                class.declare(names);
            }
        }
    }

    /// Parses the source again, and runs `check` on the tree and on the positions of its lines. Both live only
    /// for the call.
    fn with_tree<T>(&self, check: impl FnOnce(&Parsed<ModModule>, &SourceCode) -> T) -> T {
        let parsed = parse_unchecked_source(&self.source, PySourceType::Python);
        let line_index = LineIndex::from_source_text(&self.source);
        let code = SourceCode::new(&self.source, &line_index);
        check(&parsed, &code)
    }
}

fn comments<'a>(
    parsed: &'a Parsed<ModModule>,
    source: &'a str,
) -> impl Iterator<Item = (TextRange, &'a str)> {
    parsed
        .tokens()
        .into_iter()
        .filter(|token| token.kind() == TokenKind::Comment)
        .map(move |token| (token.range(), &source[token.range()]))
}

fn syntax_errors(path: &Path, source: &str, errors: &[ParseError]) -> Vec<Diagnostic> {
    let index = LineIndex::from_source_text(source);
    let code = SourceCode::new(source, &index);
    errors
        .iter()
        .map(|error| {
            let (line, col) = line_col(&code, error.location.start());
            Diagnostic {
                path: path.to_path_buf(),
                line,
                col,
                code: Code::GW000,
                message: format!("syntax error: {}", error.error),
            }
        })
        .collect()
}

pub(crate) fn sequence_elements(expr: &Expr) -> &[Expr] {
    match expr {
        Expr::Tuple(tuple) => &tuple.elts,
        Expr::List(list) => &list.elts,
        _ => &[],
    }
}

/// Converts a byte offset into a 1-based line and column.
pub(crate) fn line_col(code: &SourceCode, offset: TextSize) -> (u32, u32) {
    let position = code.line_column(offset);
    (to_u32(position.line.get()), to_u32(position.column.get()))
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::check_source;
    use crate::config::Mode;
    use crate::diagnostics::Code;

    #[test]
    fn obvious_mode_gives_a_rebound_candidate_its_place_in_source_order() {
        // `first` is judged when the module closes, after the report on line 2, but it is listed first.
        let diagnostics = check_source(
            Path::new("case.py"),
            "first = 0\nsecond = load()\nfirst = 1\n",
            Mode::Obvious,
        );
        let found: Vec<(u32, u32, Code)> = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.line, diagnostic.col, diagnostic.code))
            .collect();
        assert_eq!(found, [(1, 1, Code::GW001), (2, 1, Code::GW001)]);
    }

    #[test]
    fn obvious_class_constant_reassigned_by_a_method_is_one_gw004_naming_the_assignment() {
        let diagnostics = check_source(
            Path::new("case.py"),
            "class Limiter:\n    LIMIT = 5\n\n    def raise_limit(self) -> None:\n        self.LIMIT = 6\n",
            Mode::Obvious,
        );
        let found: Vec<(u32, u32, Code, &str)> = diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.line,
                    diagnostic.col,
                    diagnostic.code,
                    diagnostic.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [(
                2,
                5,
                Code::GW004,
                "class attribute 'LIMIT' of class 'Limiter' has no type annotation (it is reassigned on line 5)",
            )]
        );
    }

    #[test]
    fn source_that_does_not_parse_gives_its_gw000_errors() {
        let diagnostics = check_source(
            Path::new("case.py"),
            "def broken(:\n    pass\n",
            Mode::default(),
        );
        let found: Vec<(u32, u32, &str)> = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.line, diagnostic.col, diagnostic.message.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    1,
                    12,
                    "syntax error: Expected a parameter or the end of the parameter list"
                ),
                (1, 13, "syntax error: Expected `)`, found newline"),
            ]
        );
    }
}
