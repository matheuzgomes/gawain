use std::fmt;
use std::io;
use std::path::Path;

use clap::ValueEnum;
use serde::Deserialize;

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

/// Settings from the `[tool.gawain]` table of `pyproject.toml`. A key that is not a setting is an error, so
/// a misspelt key cannot be ignored without notice.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Path prefixes, relative to the working directory, whose files are skipped when walking directories.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The mode when the command line does not give one.
    #[serde(default)]
    pub mode: Mode,
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
