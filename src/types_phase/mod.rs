//! Types phase: runs the type checker on the files that presence accepted, and reads what it prints.

mod pyrefly;
mod ty;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;

use crate::config::TypeChecker;

pub use pyrefly::{PyreflyOutputError, parse_pyrefly_output};
pub use ty::{TyOutputError, parse_ty_output};

const STDERR_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// `line` and `col` are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeDiagnostic {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    pub rule: String,
    /// The description without the rule name in front of it.
    pub message: String,
    /// `Error` fails the types phase, and `Warning` does not.
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypesRun {
    pub checker: TypeChecker,
    pub diagnostics: Vec<TypeDiagnostic>,
    /// The types phase fails. For ty, the checker exited with 1, which only errors make it do, because gawain
    /// runs it with `--exit-zero-on-warning`. For pyrefly, at least one diagnostic is an error.
    pub failed: bool,
}

#[derive(Debug)]
pub enum TypesError {
    NotFound(TypeChecker),
    Start {
        path: PathBuf,
        source: io::Error,
    },
    Failed {
        checker: TypeChecker,
        code: Option<i32>,
        stderr: String,
    },
}

impl TypesError {
    fn failed(checker: TypeChecker, output: &Output) -> Self {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let head: Vec<&str> = stderr.lines().take(STDERR_LINES).collect();
        TypesError::Failed {
            checker,
            code: output.status.code(),
            stderr: head.join("\n"),
        }
    }
}

fn not_found_message(checker: TypeChecker) -> &'static str {
    match checker {
        TypeChecker::Ty => {
            "gawain: ty not found; install gawain with pip/uv (it depends on ty==0.0.100) or put ty on PATH"
        }
        TypeChecker::Pyrefly => {
            "gawain: pyrefly not found; install gawain[pyrefly] (it depends on pyrefly==1.4.1) or put pyrefly on PATH"
        }
    }
}

impl fmt::Display for TypesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypesError::NotFound(checker) => f.write_str(not_found_message(*checker)),
            TypesError::Start { path, source } => {
                write!(f, "gawain: cannot run {}: {source}", path.display())
            }
            TypesError::Failed {
                checker,
                code,
                stderr,
            } => {
                let code = code.map_or_else(|| "signal".to_owned(), |code| code.to_string());
                write!(
                    f,
                    "gawain: {} failed (exit {code}): {stderr}",
                    checker.name()
                )
            }
        }
    }
}

impl std::error::Error for TypesError {}

/// Runs `checker` on `files` from the current directory and reads what it printed. A run that gawain does not accept is
/// an error.
pub fn run(checker: TypeChecker, files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    let exe = find_checker(checker).ok_or(TypesError::NotFound(checker))?;
    match checker {
        TypeChecker::Ty => ty::run_ty(&exe, files),
        TypeChecker::Pyrefly => pyrefly::run_pyrefly(&exe, files),
    }
}

/// The `--` that ends `args` makes each file a path, so a file whose name begins with `-` is not read as an option.
fn spawn(exe: &Path, args: &[&str], files: &[PathBuf]) -> Result<Output, TypesError> {
    Command::new(exe)
        .args(args)
        .args(files)
        .output()
        .map_err(|source| TypesError::Start {
            path: exe.to_path_buf(),
            source,
        })
}

fn failed_from_exit_code(code: Option<i32>) -> Option<bool> {
    match code {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

/// The executable of `checker`: a file named after it in the directory of gawain, and otherwise the first such file on
/// `PATH`.
pub fn find_checker(checker: TypeChecker) -> Option<PathBuf> {
    let name = executable_name(checker.name());
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)));
    beside
        .filter(|candidate| candidate.is_file())
        .or_else(|| find_on_path(&name))
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
