//! Types phase: runs the type checker on the files that presence accepted, and reads what it prints.

mod pyrefly;
mod ty;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;

use crate::config::TypeChecker;

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
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypesRun {
    pub checker: TypeChecker,
    pub diagnostics: Vec<TypeDiagnostic>,
    /// Can be true without any diagnostic, when the checker's exit code alone fails the run.
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
    fn failed(checker: TypeChecker, finished: &Finished) -> Self {
        let head: Vec<&str> = finished.stderr.lines().take(STDERR_LINES).collect();
        TypesError::Failed {
            checker,
            code: finished.code,
            stderr: head.join("\n"),
        }
    }
}

fn not_found_message(checker: TypeChecker) -> &'static str {
    match checker {
        TypeChecker::Ty => {
            "gawain: ty not found; install gawain[ty] (it depends on ty==0.0.100) or put ty on PATH"
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

/// What one type checker brings to the types phase. Running it and judging the run is the same for all.
trait CheckerAdapter {
    /// Without the `--` and the files, which `spawn` adds.
    fn args(&self, files: &[PathBuf]) -> Vec<&'static str>;
    fn exit(&self, code: Option<i32>) -> Exit;
    fn parse(&self, stdout: &str) -> serde_json::Result<Vec<TypeDiagnostic>>;

    fn restore_paths(
        &self,
        diagnostics: Vec<TypeDiagnostic>,
        _files: &[PathBuf],
        _cwd: &Path,
    ) -> Vec<TypeDiagnostic> {
        diagnostics
    }
}

/// What the exit code says about the run before its diagnostics are read.
enum Exit {
    Accepted,
    Failed,
    Unexpected,
}

fn adapter_for(checker: TypeChecker) -> &'static dyn CheckerAdapter {
    match checker {
        TypeChecker::Ty => &ty::Ty,
        TypeChecker::Pyrefly => &pyrefly::Pyrefly,
    }
}

struct Finished {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl From<Output> for Finished {
    fn from(output: Output) -> Self {
        Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

/// Runs `checker` on `files` from the current directory. A run that gawain does not accept is an error.
pub fn run(checker: TypeChecker, files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    let exe = find_checker(checker).ok_or(TypesError::NotFound(checker))?;
    let finished = spawn(&exe, &adapter_for(checker).args(files), files)?;
    let cwd = std::env::current_dir().ok();
    judge(checker, &finished, files, cwd.as_deref())
}

/// Without a known `cwd`, the paths stay as the checker printed them.
fn judge(
    checker: TypeChecker,
    finished: &Finished,
    files: &[PathBuf],
    cwd: Option<&Path>,
) -> Result<TypesRun, TypesError> {
    let adapter = adapter_for(checker);
    let exit_failed = match adapter.exit(finished.code) {
        Exit::Accepted => false,
        Exit::Failed => true,
        Exit::Unexpected => return Err(TypesError::failed(checker, finished)),
    };
    let diagnostics = adapter
        .parse(&finished.stdout)
        .map_err(|_| TypesError::failed(checker, finished))?;
    let diagnostics = match cwd {
        Some(cwd) => adapter.restore_paths(diagnostics, files, cwd),
        None => diagnostics,
    };
    let failed = exit_failed
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error);
    Ok(TypesRun {
        checker,
        diagnostics,
        failed,
    })
}

/// The `--` makes each file a path, so a file whose name begins with `-` is not read as an option.
fn spawn(exe: &Path, args: &[&str], files: &[PathBuf]) -> Result<Finished, TypesError> {
    Command::new(exe)
        .args(args)
        .arg("--")
        .args(files)
        .output()
        .map(Finished::from)
        .map_err(|source| TypesError::Start {
            path: exe.to_path_buf(),
            source,
        })
}

/// The checker installed next to gawain wins over the one on `PATH`, so `pip install gawain[ty]` gets the
/// version gawain was tested with.
fn find_checker(checker: TypeChecker) -> Option<PathBuf> {
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Finished, TypesError, TypesRun, judge};
    use crate::config::TypeChecker;

    const TY_ERROR: &str = r#"[{"check_name":"invalid-assignment","description":"invalid-assignment: bad","severity":"major","location":{"path":"x.py","positions":{"begin":{"line":1,"column":1}}}}]"#;
    const TY_WARNING: &str = r#"[{"check_name":"deprecated","description":"deprecated: old","severity":"minor","location":{"path":"x.py","positions":{"begin":{"line":1,"column":1}}}}]"#;

    fn pyrefly_item(path: &str, severity: &str) -> String {
        format!(
            r#"{{"errors": [{{"line": 1, "column": 1, "path": "{path}", "name": "x", "concise_description": "y", "severity": "{severity}"}}]}}"#
        )
    }

    fn finished(code: Option<i32>, stdout: &str) -> Finished {
        Finished {
            code,
            stdout: stdout.to_owned(),
            stderr: "one\ntwo\nthree\nfour\n".to_owned(),
        }
    }

    fn judged(
        checker: TypeChecker,
        code: Option<i32>,
        stdout: &str,
    ) -> Result<TypesRun, TypesError> {
        judge(checker, &finished(code, stdout), &[], None)
    }

    fn failed(checker: TypeChecker, code: Option<i32>, stdout: &str) -> bool {
        judged(checker, code, stdout)
            .expect("run is accepted")
            .failed
    }

    #[test]
    fn ty_exit_1_fails_the_run_even_without_diagnostics() {
        assert!(failed(TypeChecker::Ty, Some(1), "[]"));
    }

    #[test]
    fn ty_warning_with_exit_0_passes() {
        assert!(!failed(TypeChecker::Ty, Some(0), TY_WARNING));
    }

    #[test]
    fn an_error_diagnostic_fails_the_run_whatever_the_exit_code() {
        assert!(failed(TypeChecker::Ty, Some(0), TY_ERROR));
        assert!(failed(
            TypeChecker::Pyrefly,
            Some(1),
            &pyrefly_item("x.py", "error")
        ));
    }

    #[test]
    fn pyrefly_exit_1_with_only_warnings_passes() {
        assert!(!failed(
            TypeChecker::Pyrefly,
            Some(1),
            &pyrefly_item("x.py", "warn")
        ));
    }

    #[test]
    fn unexpected_exit_code_is_an_error_with_the_head_of_stderr() {
        for checker in [TypeChecker::Ty, TypeChecker::Pyrefly] {
            let error = judged(checker, Some(2), "[]").expect_err("exit 2 is not accepted");
            assert!(matches!(
                error,
                TypesError::Failed { code: Some(2), ref stderr, .. } if stderr == "one\ntwo\nthree"
            ));
        }
    }

    #[test]
    fn output_that_does_not_parse_is_an_error_with_the_head_of_stderr() {
        for (checker, stdout) in [(TypeChecker::Ty, "not json"), (TypeChecker::Pyrefly, "[]")] {
            let error = judged(checker, Some(0), stdout).expect_err("output does not parse");
            assert!(matches!(
                error,
                TypesError::Failed { code: Some(0), ref stderr, .. } if stderr == "one\ntwo\nthree"
            ));
        }
    }

    #[test]
    fn a_checker_killed_by_a_signal_is_an_error() {
        for checker in [TypeChecker::Ty, TypeChecker::Pyrefly] {
            let error = judged(checker, None, "[]").expect_err("no exit code is not accepted");
            assert!(matches!(error, TypesError::Failed { code: None, .. }));
        }
    }

    #[test]
    fn pyrefly_paths_are_restored_against_the_working_directory() {
        let files = [PathBuf::from("./z.py")];
        let stdout = pyrefly_item("z.py", "warn");
        let restored = judge(
            TypeChecker::Pyrefly,
            &finished(Some(0), &stdout),
            &files,
            Some(Path::new("/proj")),
        );
        let kept = judge(
            TypeChecker::Pyrefly,
            &finished(Some(0), &stdout),
            &files,
            None,
        );

        assert_eq!(
            restored.expect("accepted").diagnostics[0].path,
            Path::new("./z.py")
        );
        assert_eq!(
            kept.expect("accepted").diagnostics[0].path,
            Path::new("z.py")
        );
    }
}
