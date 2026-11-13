use std::fmt;
use std::io;
use std::path::Path;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// How the presence phase judges a first binding whose value is obvious, such as `count = 0`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// An obvious first binding needs no annotation. See docs/rules.md for the rules.
    #[default]
    Obvious,
    /// Every first binding needs an annotation.
    Strict,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TypeChecker {
    /// Astral's ty, installed with `gawain[ty]`.
    #[default]
    Ty,
    /// Meta's pyrefly, installed with `gawain[pyrefly]`.
    Pyrefly,
}

impl TypeChecker {
    /// The checker's name, as the user writes it and as reports print it: `ty` or `pyrefly`.
    pub fn name(self) -> &'static str {
        match self {
            TypeChecker::Ty => "ty",
            TypeChecker::Pyrefly => "pyrefly",
        }
    }
}

/// Settings from the `[tool.gawain]` table of `pyproject.toml`. A key that is not a setting is an error, so
/// a misspelt key cannot be ignored without notice.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Path prefixes, relative to the working directory, whose files are skipped when walking directories.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub checker: TypeChecker,
}

#[derive(Deserialize)]
struct PyProject {
    tool: Option<Tool>,
}

#[derive(Deserialize)]
struct Tool {
    gawain: Option<Config>,
}

/// The configuration file exists but cannot be read or parsed. The text starts with the file path.
#[derive(Debug)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(path: &Path, cause: impl fmt::Display) -> Self {
        Self {
            message: format!("{}: {cause}", path.display()),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

const FILE_NAME: &str = "pyproject.toml";

/// Reads `[tool.gawain]` from `./pyproject.toml`. A missing file or a missing section gives the default config.
pub fn load() -> Result<Config, ConfigError> {
    load_from(Path::new(FILE_NAME))
}

fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let Some(text) = read_if_present(path)? else {
        return Ok(Config::default());
    };
    let project: PyProject =
        toml::from_str(&text).map_err(|error| ConfigError::new(path, error))?;
    Ok(project
        .tool
        .and_then(|tool| tool.gawain)
        .unwrap_or_default())
}

/// The text of the file at `path`, or `None` when there is no such file. Any other read failure is an error.
fn read_if_present(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ConfigError::new(path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, TypeChecker};

    #[test]
    fn checker_key_selects_pyrefly() {
        let config: Config = toml::from_str("checker = \"pyrefly\"").expect("valid settings");
        assert_eq!(config.checker, TypeChecker::Pyrefly);
    }

    #[test]
    fn missing_checker_key_selects_ty() {
        let config: Config = toml::from_str("").expect("empty settings are valid");
        assert_eq!(config.checker, TypeChecker::Ty);
    }

    #[test]
    fn unknown_checker_is_an_error() {
        let error =
            toml::from_str::<Config>("checker = \"mypy\"").expect_err("mypy is not a checker");
        assert!(error.to_string().contains("mypy"), "error was: {error}");
    }

    #[test]
    fn checker_names_are_ty_and_pyrefly() {
        assert_eq!(TypeChecker::Ty.name(), "ty");
        assert_eq!(TypeChecker::Pyrefly.name(), "pyrefly");
    }
}
