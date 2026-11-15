//! One `gawain check`: presence on every file, then the types phase once presence is clean.

use std::fmt;

use rayon::ThreadPoolBuildError;

use crate::config::{Mode, TypeChecker};
use crate::diagnostics::Diagnostic;
use crate::discovery::Discovery;
use crate::presence;
use crate::types_phase::{self, TypesError, TypesRun};

#[derive(Debug)]
pub struct CheckRun {
    pub presence: Vec<Diagnostic>,
    /// `None` when the types phase did not run: presence found a problem, it was turned off, or no file was found.
    pub types: Option<TypesRun>,
}

impl CheckRun {
    pub fn passed(&self) -> bool {
        self.presence.is_empty() && !self.types.as_ref().is_some_and(|run| run.failed)
    }
}

#[derive(Debug)]
pub enum CheckError {
    Presence(ThreadPoolBuildError),
    Types(TypesError),
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::Presence(error) => write!(f, "gawain: {error}"),
            CheckError::Types(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CheckError {}

/// `checker` is `None` when the types phase is turned off.
pub fn run(
    found: Discovery,
    mode: Mode,
    checker: Option<TypeChecker>,
) -> Result<CheckRun, CheckError> {
    let presence =
        presence::check_files(&found.files, found.errors, mode).map_err(CheckError::Presence)?;
    let types = match checker {
        Some(checker) if presence.is_empty() && !found.files.is_empty() => {
            Some(types_phase::run(checker, &found.files).map_err(CheckError::Types)?)
        }
        _ => None,
    };
    Ok(CheckRun { presence, types })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{CheckRun, run};
    use crate::config::{Mode, TypeChecker};
    use crate::diagnostics::Diagnostic;
    use crate::discovery::Discovery;
    use crate::types_phase::TypesRun;

    fn fully_annotated() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fully_annotated.py")
    }

    fn found(files: Vec<PathBuf>, errors: Vec<Diagnostic>) -> Discovery {
        Discovery { files, errors }
    }

    fn walk_error() -> Diagnostic {
        Diagnostic::for_path(Path::new("unreadable"), "cannot read directory".to_owned())
    }

    fn types_run(failed: bool) -> Option<TypesRun> {
        Some(TypesRun {
            checker: TypeChecker::Ty,
            diagnostics: Vec::new(),
            failed,
        })
    }

    #[test]
    fn types_phase_does_not_run_while_presence_has_a_problem() {
        let found = found(vec![fully_annotated()], vec![walk_error()]);
        let run = run(found, Mode::Strict, Some(TypeChecker::Ty)).expect("check runs");
        assert_eq!(run.presence.len(), 1);
        assert!(run.types.is_none());
    }

    #[test]
    fn types_phase_does_not_run_without_files() {
        let run = run(
            found(Vec::new(), Vec::new()),
            Mode::Strict,
            Some(TypeChecker::Ty),
        )
        .expect("check runs");
        assert!(run.types.is_none());
    }

    #[test]
    fn types_phase_does_not_run_when_turned_off() {
        let run = run(
            found(vec![fully_annotated()], Vec::new()),
            Mode::Strict,
            None,
        )
        .expect("check runs");
        assert!(run.presence.is_empty());
        assert!(run.types.is_none());
    }

    #[test]
    fn a_failed_types_phase_or_a_presence_problem_fails_the_check() {
        let clean = CheckRun {
            presence: Vec::new(),
            types: None,
        };
        let warnings_only = CheckRun {
            presence: Vec::new(),
            types: types_run(false),
        };
        let types_failed = CheckRun {
            presence: Vec::new(),
            types: types_run(true),
        };
        let presence_problem = CheckRun {
            presence: vec![walk_error()],
            types: None,
        };

        assert!(clean.passed());
        assert!(warnings_only.passed());
        assert!(!types_failed.passed());
        assert!(!presence_problem.passed());
    }
}
