//! Types phase: runs the ty type checker on the files that presence accepted, and reads what it prints.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};

/// The `severity` that ty 0.0.100 gives a warning in gitlab output. Every other severity is an error.
pub const WARNING_SEVERITY: &str = "minor";

/// How many lines of ty's stderr an error message shows.
const STDERR_LINES: usize = 3;

/// One diagnostic that ty printed. `line` and `col` are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeDiagnostic {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    /// ty's rule name, such as `invalid-assignment`.
    pub rule: String,
    /// The description without the rule name in front of it.
    pub message: String,
    /// ty's severity, such as `major` for an error or `minor` for a warning.
    pub severity: String,
}

/// A types phase run that ty finished: every diagnostic it printed, and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypesRun {
    pub diagnostics: Vec<TypeDiagnostic>,
    /// ty exited with 1, so the types phase fails. Only errors make ty exit with 1, because gawain runs
    /// it with `--exit-zero-on-warning`.
    pub failed: bool,
}

/// Why the types phase could not finish.
#[derive(Debug)]
pub enum TypesError {
    NotFound,
    Start { path: PathBuf, source: io::Error },
    Failed { code: Option<i32>, stderr: String },
}

impl TypesError {
    /// The failure of a ty run whose exit code or stdout gawain does not accept.
    fn failed(output: &Output) -> Self {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let head: Vec<&str> = stderr.lines().take(STDERR_LINES).collect();
        TypesError::Failed {
            code: output.status.code(),
            stderr: head.join("\n"),
        }
    }
}

impl fmt::Display for TypesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypesError::NotFound => f.write_str(
                "gawain: ty not found; install gawain with pip/uv (it depends on ty==0.0.100) or put ty on PATH",
            ),
            TypesError::Start { path, source } => {
                write!(f, "gawain: cannot run {}: {source}", path.display())
            }
            TypesError::Failed { code, stderr } => {
                let code = code.map_or_else(|| "signal".to_owned(), |code| code.to_string());
                write!(f, "gawain: ty failed (exit {code}): {stderr}")
            }
        }
    }
}

impl std::error::Error for TypesError {}

/// ty's output is not the gitlab JSON array that gawain reads.
#[derive(Debug)]
pub struct TyOutputError(serde_json::Error);

impl fmt::Display for TyOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ty output is not gitlab JSON: {}", self.0)
    }
}

impl std::error::Error for TyOutputError {}

/// Runs ty on `files` from the current directory. Exit 0 is a clean run and 1 a failed one, where ty
/// exits 0 when it finds only warnings. Any other exit code, or stdout that is not gitlab JSON, is an
/// error.
pub fn run(files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    let ty = find_ty().ok_or(TypesError::NotFound)?;
    let output = invoke(&ty, files)?;
    interpret(&output)
}

/// Starts `ty` on `files` in gitlab format, with warnings that do not fail the run. The `--` before the
/// files makes each of them a path, so a file whose name begins with `-` is not read as an option.
fn invoke(ty: &Path, files: &[PathBuf]) -> Result<Output, TypesError> {
    Command::new(ty)
        .args([
            "check",
            "--output-format",
            "gitlab",
            "--exit-zero-on-warning",
            "--",
        ])
        .args(files)
        .output()
        .map_err(|source| TypesError::Start {
            path: ty.to_path_buf(),
            source,
        })
}

/// Reads what `ty` printed. Its exit code says whether the run failed, and its stdout must be gitlab JSON.
fn interpret(output: &Output) -> Result<TypesRun, TypesError> {
    let failed =
        failed_from_exit_code(output.status.code()).ok_or_else(|| TypesError::failed(output))?;
    let diagnostics = parse_ty_output(&String::from_utf8_lossy(&output.stdout))
        .map_err(|_| TypesError::failed(output))?;
    Ok(TypesRun {
        diagnostics,
        failed,
    })
}

/// Whether a run failed, from its exit code: 0 is clean and 1 is failed. Any other code is not accepted.
fn failed_from_exit_code(code: Option<i32>) -> Option<bool> {
    match code {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

/// The ty binary: a file named `ty` (`ty.exe` on Windows) in the directory of gawain, and otherwise the
/// first such file on `PATH`.
pub fn find_ty() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ty.exe" } else { "ty" };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    beside
        .filter(|candidate| candidate.is_file())
        .or_else(|| find_on_path(name))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Reads ty's gitlab output: a JSON array with one item per diagnostic. An empty array is a clean run.
pub fn parse_ty_output(stdout: &str) -> Result<Vec<TypeDiagnostic>, TyOutputError> {
    let items: Vec<GitlabItem> = serde_json::from_str(stdout).map_err(TyOutputError)?;
    Ok(items.into_iter().map(TypeDiagnostic::from).collect())
}

/// One item of ty's gitlab output. Fields that gawain does not use are ignored.
#[derive(Deserialize)]
struct GitlabItem {
    check_name: String,
    description: String,
    severity: String,
    location: GitlabLocation,
}

#[derive(Deserialize)]
struct GitlabLocation {
    path: PathBuf,
    positions: GitlabPositions,
}

#[derive(Deserialize)]
struct GitlabPositions {
    begin: GitlabPosition,
}

#[derive(Deserialize)]
struct GitlabPosition {
    line: u32,
    column: u32,
}

impl From<GitlabItem> for TypeDiagnostic {
    fn from(item: GitlabItem) -> Self {
        let prefix = format!("{}: ", item.check_name);
        let message = item
            .description
            .strip_prefix(prefix.as_str())
            .unwrap_or(&item.description)
            .to_owned();
        let begin = item.location.positions.begin;
        TypeDiagnostic {
            path: item.location.path,
            line: begin.line,
            col: begin.column,
            rule: item.check_name,
            message,
            severity: item.severity,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{TypeDiagnostic, parse_ty_output};

    /// What ty 0.0.100 printed for `x: int = "a"` in `bad.py`.
    const SAMPLE: &str = r#"[
  {
    "check_name": "invalid-assignment",
    "description": "invalid-assignment: Object of type `Literal[\"a\"]` is not assignable to `int`",
    "severity": "major",
    "fingerprint": "966195585497f407",
    "location": {
      "path": "bad.py",
      "positions": {
        "begin": {"line": 1, "column": 10},
        "end": {"line": 1, "column": 13}
      }
    }
  }
]"#;

    #[test]
    fn sample_gives_rule_message_and_one_based_position() {
        assert_eq!(
            parse_ty_output(SAMPLE).ok(),
            Some(vec![TypeDiagnostic {
                path: PathBuf::from("bad.py"),
                line: 1,
                col: 10,
                rule: "invalid-assignment".to_owned(),
                message: "Object of type `Literal[\"a\"]` is not assignable to `int`".to_owned(),
                severity: "major".to_owned(),
            }])
        );
    }

    #[test]
    fn empty_array_is_a_clean_run() {
        assert_eq!(parse_ty_output("[]").ok(), Some(Vec::new()));
    }

    #[test]
    fn text_that_is_not_json_is_an_error() {
        assert!(parse_ty_output("not json").is_err());
    }

    #[test]
    fn output_error_says_the_output_is_not_gitlab_json() {
        let message = parse_ty_output("not json")
            .err()
            .map(|error| error.to_string());
        assert!(message.is_some_and(|text| text.starts_with("ty output is not gitlab JSON: ")));
    }

    #[test]
    fn description_without_the_rule_prefix_is_kept_whole() {
        let stdout = r#"[{"check_name":"deprecated","description":"other: text","severity":"minor","location":{"path":"x.py","positions":{"begin":{"line":2,"column":3}}}}]"#;
        assert_eq!(
            parse_ty_output(stdout).ok(),
            Some(vec![TypeDiagnostic {
                path: PathBuf::from("x.py"),
                line: 2,
                col: 3,
                rule: "deprecated".to_owned(),
                message: "other: text".to_owned(),
                severity: "minor".to_owned(),
            }])
        );
    }
}
