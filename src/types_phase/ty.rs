//! The ty checker: runs `ty check` and reads its gitlab JSON output.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde::Deserialize;

use crate::config::TypeChecker;

use super::{Severity, TypeDiagnostic, TypesError, TypesRun, failed_from_exit_code, spawn};

/// The gitlab `severity` that ty 0.0.100 gives a warning. Every other severity is an error.
const TY_WARNING_SEVERITY: &str = "minor";

/// ty's output is not the gitlab JSON array that gawain reads.
#[derive(Debug)]
pub struct TyOutputError(serde_json::Error);

impl fmt::Display for TyOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ty output is not gitlab JSON: {}", self.0)
    }
}

impl std::error::Error for TyOutputError {}

/// Runs ty on `files` in gitlab format, with warnings that do not fail the run.
pub(super) fn run_ty(ty: &Path, files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    let args = [
        "check",
        "--output-format",
        "gitlab",
        "--exit-zero-on-warning",
        "--",
    ];
    let output = spawn(ty, &args, files)?;
    interpret_ty(&output)
}

/// Reads ty's output. Its exit code says whether the run failed, and its stdout must be gitlab JSON.
fn interpret_ty(output: &Output) -> Result<TypesRun, TypesError> {
    let failed = failed_from_exit_code(output.status.code())
        .ok_or_else(|| TypesError::failed(TypeChecker::Ty, output))?;
    let diagnostics = parse_ty_output(&String::from_utf8_lossy(&output.stdout))
        .map_err(|_| TypesError::failed(TypeChecker::Ty, output))?;
    Ok(TypesRun {
        checker: TypeChecker::Ty,
        diagnostics,
        failed,
    })
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
            severity: severity_of(&item.severity),
        }
    }
}

/// The severity of a ty diagnostic: `minor` is a warning, and every other gitlab severity is an error.
fn severity_of(gitlab: &str) -> Severity {
    if gitlab == TY_WARNING_SEVERITY {
        Severity::Warning
    } else {
        Severity::Error
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse_ty_output;
    use crate::types_phase::{Severity, TypeDiagnostic};

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
                severity: Severity::Error,
            }])
        );
    }

    #[test]
    fn every_severity_but_minor_is_an_error() {
        let stdout = r#"[{"check_name":"x","description":"x: y","severity":"critical","location":{"path":"x.py","positions":{"begin":{"line":1,"column":1}}}}]"#;
        let diagnostics = parse_ty_output(stdout).expect("valid gitlab JSON");
        assert_eq!(diagnostics[0].severity, Severity::Error);
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
                severity: Severity::Warning,
            }])
        );
    }
}
