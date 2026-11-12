//! The pyrefly checker: runs `pyrefly check` and reads its JSON output.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf, absolute};
use std::process::Output;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::TypeChecker;

use super::{Severity, TypeDiagnostic, TypesError, TypesRun, spawn};

/// pyrefly's output is not the JSON object with an `errors` array that gawain reads.
#[derive(Debug)]
pub struct PyreflyOutputError(serde_json::Error);

impl fmt::Display for PyreflyOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pyrefly output is not JSON with an errors array: {}",
            self.0
        )
    }
}

impl std::error::Error for PyreflyOutputError {}

/// Runs pyrefly on `files` in JSON format and reads what it printed. `--preset default` is added when no pyrefly
/// config applies to the files, because pyrefly's built-in preset reports too little. With a config, the flag
/// would override the project's own preset, so it is left out.
pub(super) fn run_pyrefly(pyrefly: &Path, files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    let mut args = vec![
        "check",
        "--output-format",
        "json",
        "--summary=none",
        "--min-severity",
        "warn",
    ];
    if !has_pyrefly_config(files) {
        args.extend(["--preset", "default"]);
    }
    args.push("--");
    let output = spawn(pyrefly, &args, files)?;
    interpret_pyrefly(&output, files)
}

/// Reads pyrefly's output. Exit 0 and 1 are both accepted, because pyrefly exits 1 for warnings too. The run
/// fails only when one of its items is an error. The paths in the items are mapped back to the files in `files`.
fn interpret_pyrefly(output: &Output, files: &[PathBuf]) -> Result<TypesRun, TypesError> {
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(TypesError::failed(TypeChecker::Pyrefly, output));
    }
    let diagnostics = parse_pyrefly_output(&String::from_utf8_lossy(&output.stdout))
        .map_err(|_| TypesError::failed(TypeChecker::Pyrefly, output))?;
    let diagnostics = restore_passed_paths(diagnostics, files, std::env::current_dir());
    let failed = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    Ok(TypesRun {
        checker: TypeChecker::Pyrefly,
        diagnostics,
        failed,
    })
}
/// Reads pyrefly's JSON output: one object whose `errors` array has one item per diagnostic. An empty array is a
/// clean run. An array is not that object, even when it holds one.
pub fn parse_pyrefly_output(stdout: &str) -> Result<Vec<TypeDiagnostic>, PyreflyOutputError> {
    // A map rejects an array. Reading the report straight from the text would take an array for its fields.
    let object: Map<String, Value> = serde_json::from_str(stdout).map_err(PyreflyOutputError)?;
    let report: PyreflyReport =
        serde_json::from_value(Value::Object(object)).map_err(PyreflyOutputError)?;
    Ok(report
        .errors
        .into_iter()
        .map(TypeDiagnostic::from)
        .collect())
}

/// pyrefly's JSON report. Fields that gawain does not use are ignored.
#[derive(Deserialize)]
struct PyreflyReport {
    errors: Vec<PyreflyItem>,
}

/// One item of pyrefly's JSON report. Fields that gawain does not use are ignored.
#[derive(Deserialize)]
struct PyreflyItem {
    line: u32,
    column: u32,
    path: PathBuf,
    name: String,
    concise_description: String,
    severity: String,
}

impl From<PyreflyItem> for TypeDiagnostic {
    fn from(item: PyreflyItem) -> Self {
        TypeDiagnostic {
            path: item.path,
            line: item.line,
            col: item.column,
            rule: item.name,
            message: item.concise_description,
            severity: pyrefly_severity(&item.severity),
        }
    }
}

/// The severity of a pyrefly item: `error` is an error, and `warn` and `info` are warnings.
fn pyrefly_severity(severity: &str) -> Severity {
    // --min-severity warn means pyrefly prints only error, warn and info, so anything but error is a warning.
    if severity == "error" {
        Severity::Error
    } else {
        Severity::Warning
    }
}

/// Whether a pyrefly config applies to one of `files`. A config applies to a file when the file's directory, or
/// a directory above it, holds one. The walk goes over the folded path, so `..` names the directory above.
fn has_pyrefly_config(files: &[PathBuf]) -> bool {
    let mut checked = HashSet::new();
    files
        .iter()
        .filter_map(|file| lexical_absolute(file).parent().map(Path::to_path_buf))
        .any(|dir| has_config_above(&dir, &mut checked))
}

/// Whether `dir` or a directory above it holds a pyrefly config. A directory in `checked` was walked by an
/// earlier file and holds no config, so it is not checked again.
fn has_config_above(dir: &Path, checked: &mut HashSet<PathBuf>) -> bool {
    dir.ancestors()
        .any(|ancestor| checked.insert(ancestor.to_path_buf()) && pyrefly_config_in(ancestor))
}

/// `path` made absolute and folded lexically: `.` is dropped, and `..` removes the component before it without
/// going above the root. Symbolic links are not followed, because pyrefly does not follow them either. A path
/// that cannot be made absolute is kept as it is.
fn lexical_absolute(path: &Path) -> PathBuf {
    let made_absolute = absolute(path).unwrap_or_else(|_| path.to_path_buf());
    made_absolute
        .components()
        .fold(PathBuf::new(), fold_component)
}

/// `folded` with `component` added, except that `..` removes the last component instead. The components of a
/// path have no `.` in them, because `Path::components` drops those.
fn fold_component(mut folded: PathBuf, component: Component<'_>) -> PathBuf {
    if component == Component::ParentDir {
        folded.pop();
    } else {
        folded.push(component);
    }
    folded
}

/// `diagnostics` with each path mapped back to the file that gawain passed. pyrefly prints every path relative to
/// its working directory, and an absolute path outside it without the leading `/`. A path names a passed file when
/// the file is that path from `cwd`, or from `/` on unix. A path that names no passed file is kept as pyrefly
/// printed it.
fn restore_paths(
    diagnostics: Vec<TypeDiagnostic>,
    files: &[PathBuf],
    cwd: &Path,
) -> Vec<TypeDiagnostic> {
    let passed: HashMap<PathBuf, &Path> = files
        .iter()
        .map(|file| (lexical_absolute(&cwd.join(file)), file.as_path()))
        .collect();
    diagnostics
        .into_iter()
        .map(|diagnostic| restore_path(diagnostic, &passed, cwd))
        .collect()
}

/// `diagnostic` with its path replaced by the passed file that the path names, when one does.
fn restore_path(
    mut diagnostic: TypeDiagnostic,
    passed: &HashMap<PathBuf, &Path>,
    cwd: &Path,
) -> TypeDiagnostic {
    let file = candidate_paths(&diagnostic.path, cwd)
        .iter()
        .find_map(|candidate| passed.get(candidate).copied());
    if let Some(file) = file {
        diagnostic.path = file.to_path_buf();
    }
    diagnostic
}

/// The absolute paths that a path printed by pyrefly can name: the path from `cwd`, and the path from `/`, because
/// pyrefly drops the `/` of an absolute path outside its working directory.
#[cfg(unix)]
fn candidate_paths(printed: &Path, cwd: &Path) -> Vec<PathBuf> {
    vec![
        lexical_absolute(&cwd.join(printed)),
        lexical_absolute(&Path::new("/").join(printed)),
    ]
}

/// The absolute path that a path printed by pyrefly can name: the path from `cwd`. Off unix, `/` is not a root,
/// so the path from `/` is not a candidate.
#[cfg(not(unix))]
fn candidate_paths(printed: &Path, cwd: &Path) -> Vec<PathBuf> {
    vec![lexical_absolute(&cwd.join(printed))]
}

/// `restore_paths` against the working directory `cwd`, which the caller reads. When that is unknown, the
/// diagnostics keep the paths that pyrefly printed.
fn restore_passed_paths(
    diagnostics: Vec<TypeDiagnostic>,
    files: &[PathBuf],
    cwd: io::Result<PathBuf>,
) -> Vec<TypeDiagnostic> {
    match cwd {
        Ok(cwd) => restore_paths(diagnostics, files, &cwd),
        Err(_) => diagnostics,
    }
}

/// Whether `dir` holds a `pyrefly.toml`, or a `pyproject.toml` with a `[tool.pyrefly]` table.
fn pyrefly_config_in(dir: &Path) -> bool {
    dir.join("pyrefly.toml").is_file() || pyproject_has_pyrefly_table(&dir.join("pyproject.toml"))
}

/// The parts of a `pyproject.toml` that the pyrefly config check reads.
#[derive(Deserialize)]
struct PyProjectTools {
    tool: Option<ToolTables>,
}

#[derive(Deserialize)]
struct ToolTables {
    pyrefly: Option<toml::Table>,
}

/// Whether the `pyproject.toml` at `path` has a `[tool.pyrefly]` table. A file that cannot be read or parsed
/// has none.
fn pyproject_has_pyrefly_table(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    toml::from_str::<PyProjectTools>(&text)
        .is_ok_and(|project| project.tool.and_then(|tool| tool.pyrefly).is_some())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{has_pyrefly_config, lexical_absolute, parse_pyrefly_output, restore_passed_paths};
    use crate::types_phase::{Severity, TypeDiagnostic};

    /// What pyrefly 1.4.1 printed for `x: int = "a"` in `plain.py`.
    const PYREFLY_SAMPLE: &str = r#"{
  "errors": [
    {
      "line": 1,
      "column": 10,
      "stop_line": 1,
      "stop_column": 13,
      "path": "plain.py",
      "code": -2,
      "name": "bad-assignment",
      "description": "`Literal['a']` is not assignable to `int`",
      "concise_description": "`Literal['a']` is not assignable to `int`",
      "severity": "error"
    }
  ]
}"#;

    /// A pyrefly report with one item of `severity` at line 2, column 3 of `x.py`. Its description has two
    /// lines, and its concise description is `short`.
    fn pyrefly_report_with(severity: &str) -> String {
        format!(
            r#"{{"errors": [{{"line": 2, "column": 3, "path": "x.py", "name": "deprecated", "description": "long\nlines", "concise_description": "short", "severity": "{severity}"}}]}}"#
        )
    }

    #[test]
    fn pyrefly_sample_gives_rule_message_and_one_based_position() {
        assert_eq!(
            parse_pyrefly_output(PYREFLY_SAMPLE).ok(),
            Some(vec![TypeDiagnostic {
                path: PathBuf::from("plain.py"),
                line: 1,
                col: 10,
                rule: "bad-assignment".to_owned(),
                message: "`Literal['a']` is not assignable to `int`".to_owned(),
                severity: Severity::Error,
            }])
        );
    }

    #[test]
    fn pyrefly_message_is_the_one_line_concise_description() {
        let diagnostics =
            parse_pyrefly_output(&pyrefly_report_with("error")).expect("valid pyrefly JSON");
        assert_eq!(diagnostics[0].message, "short");
    }

    #[test]
    fn pyrefly_empty_errors_array_is_a_clean_run() {
        assert_eq!(
            parse_pyrefly_output("{\n  \"errors\": []\n}\n").ok(),
            Some(Vec::new())
        );
    }

    #[test]
    fn pyrefly_error_severity_is_error() {
        let diagnostics =
            parse_pyrefly_output(&pyrefly_report_with("error")).expect("valid pyrefly JSON");
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn pyrefly_warn_and_info_are_warnings() {
        for severity in ["warn", "info"] {
            let diagnostics =
                parse_pyrefly_output(&pyrefly_report_with(severity)).expect("valid pyrefly JSON");
            assert_eq!(
                diagnostics[0].severity,
                Severity::Warning,
                "severity {severity}"
            );
        }
    }

    #[test]
    fn pyrefly_text_that_is_not_json_is_an_error() {
        assert!(parse_pyrefly_output("not json").is_err());
    }

    #[test]
    fn pyrefly_json_array_is_not_the_report() {
        assert!(parse_pyrefly_output("[]").is_err());
        assert!(parse_pyrefly_output("[[]]").is_err());
    }

    #[test]
    fn pyrefly_object_without_errors_is_an_error() {
        assert!(parse_pyrefly_output("{}").is_err());
    }

    #[test]
    fn pyrefly_output_error_says_the_output_is_not_json_with_an_errors_array() {
        let message = parse_pyrefly_output("not json")
            .err()
            .map(|error| error.to_string());
        assert!(message.is_some_and(|text| {
            text.starts_with("pyrefly output is not JSON with an errors array: ")
        }));
    }

    /// A directory in the system temp directory that is removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after 1970")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("gawain-unit-{}-{nanos}-{name}", std::process::id()));
            fs::create_dir_all(&path).expect("temp directory is creatable");
            TempDir(path)
        }

        /// Writes `text` to `relative` under this directory, creating parent directories, and returns its path.
        fn file(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(
                path.parent()
                    .expect("a file under the directory has a parent"),
            )
            .expect("temp directory is creatable");
            fs::write(&path, text).expect("temp file is writable");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            // Nothing is left to remove when the test has already removed the directory.
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_pyrefly_config_is_not_a_config() {
        let dir = TempDir::new("no-config");
        let file = dir.file("sub/case.py", "");
        assert!(!has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyrefly_toml_next_to_the_file_is_a_config() {
        let dir = TempDir::new("toml-beside");
        dir.file("pyrefly.toml", "");
        let file = dir.file("case.py", "");
        assert!(has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyrefly_toml_in_a_parent_directory_is_a_config() {
        let dir = TempDir::new("toml-above");
        dir.file("pyrefly.toml", "");
        let file = dir.file("pkg/sub/case.py", "");
        assert!(has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyproject_with_a_pyrefly_table_is_a_config() {
        let dir = TempDir::new("table");
        dir.file("pyproject.toml", "[tool.pyrefly]\n");
        let file = dir.file("case.py", "");
        assert!(has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyproject_without_a_pyrefly_table_is_not_a_config() {
        let dir = TempDir::new("other-table");
        dir.file("pyproject.toml", "[tool.gawain]\n");
        let file = dir.file("case.py", "");
        assert!(!has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyproject_without_a_pyrefly_table_lets_the_walk_go_on_upward() {
        let dir = TempDir::new("walk-on");
        dir.file("pyrefly.toml", "");
        dir.file("sub/pyproject.toml", "[tool.gawain]\n");
        let file = dir.file("sub/case.py", "");
        assert!(has_pyrefly_config(&[file]));
    }

    #[test]
    fn unparsable_pyproject_is_not_a_config() {
        let dir = TempDir::new("broken");
        dir.file("pyproject.toml", "[tool.pyrefly\n");
        let file = dir.file("case.py", "");
        assert!(!has_pyrefly_config(&[file]));
    }

    #[test]
    fn pyproject_that_cannot_be_read_is_not_a_config() {
        let dir = TempDir::new("unreadable");
        fs::create_dir(dir.0.join("pyproject.toml")).expect("temp directory is creatable");
        let file = dir.file("case.py", "");
        assert!(!has_pyrefly_config(&[file]));
    }

    #[test]
    fn one_file_under_a_config_is_enough() {
        let dir = TempDir::new("one-of-two");
        dir.file("with/pyproject.toml", "[tool.pyrefly]\n");
        let with = dir.file("with/a.py", "");
        let without = dir.file("without/b.py", "");
        assert!(has_pyrefly_config(&[without, with]));
    }

    #[test]
    fn files_in_a_directory_without_a_config_are_not_a_config() {
        let dir = TempDir::new("two-plain");
        let first = dir.file("sub/a.py", "");
        let second = dir.file("sub/b.py", "");
        assert!(!has_pyrefly_config(&[first, second]));
    }

    #[cfg(unix)]
    #[test]
    fn lexical_absolute_folds_parent_components() {
        assert_eq!(
            lexical_absolute(Path::new("/a/b/../c/x.py")),
            PathBuf::from("/a/c/x.py")
        );
    }

    #[cfg(unix)]
    #[test]
    fn lexical_absolute_drops_current_directory_components() {
        assert_eq!(
            lexical_absolute(Path::new("/a/./b/x.py")),
            PathBuf::from("/a/b/x.py")
        );
    }

    #[cfg(unix)]
    #[test]
    fn lexical_absolute_never_climbs_above_the_root() {
        assert_eq!(
            lexical_absolute(Path::new("/../x.py")),
            PathBuf::from("/x.py")
        );
    }

    #[test]
    fn lexical_absolute_keeps_a_path_that_cannot_be_made_absolute() {
        assert_eq!(lexical_absolute(Path::new("")), PathBuf::new());
    }

    #[test]
    fn parent_component_does_not_walk_through_the_directory_it_leaves() {
        let dir = TempDir::new("parent-walk");
        dir.file("app/pyproject.toml", "[tool.pyrefly]\n");
        dir.file("other/x.py", "");
        let file = dir.0.join("app/../other/x.py");
        assert!(!has_pyrefly_config(&[file]));
    }

    /// A diagnostic that pyrefly printed for `path`.
    fn diagnostic_at(path: &str) -> TypeDiagnostic {
        TypeDiagnostic {
            path: PathBuf::from(path),
            line: 1,
            col: 10,
            rule: "bad-assignment".to_owned(),
            message: "`Literal['a']` is not assignable to `int`".to_owned(),
            severity: Severity::Error,
        }
    }

    /// The path that gawain gives a diagnostic which pyrefly printed as `printed`, when gawain passed `files`
    /// from the working directory `cwd`.
    fn restored(printed: &str, files: &[&str], cwd: &str) -> PathBuf {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        restore_passed_paths(vec![diagnostic_at(printed)], &files, Ok(PathBuf::from(cwd)))
            .remove(0)
            .path
    }

    #[test]
    fn a_file_passed_as_dot_slash_is_restored_to_dot_slash() {
        assert_eq!(
            restored("z.py", &["./z.py"], "/proj/app"),
            PathBuf::from("./z.py")
        );
    }

    #[test]
    fn a_parent_relative_path_is_restored_to_the_absolute_file_passed() {
        assert_eq!(
            restored("../other/x.py", &["/proj/other/x.py"], "/proj/app"),
            PathBuf::from("/proj/other/x.py")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_the_root_slash_dropped_is_restored_to_the_absolute_file_passed() {
        assert_eq!(
            restored("tmp/q/x.py", &["/tmp/q/x.py"], "/home/u/proj"),
            PathBuf::from("/tmp/q/x.py")
        );
    }

    #[test]
    fn a_path_that_matches_no_file_passed_is_kept() {
        assert_eq!(
            restored("other.py", &["/proj/app/z.py"], "/proj/app"),
            PathBuf::from("other.py")
        );
    }

    #[test]
    fn paths_are_kept_when_the_working_directory_is_unknown() {
        let diagnostics = vec![diagnostic_at("z.py")];
        let files = [PathBuf::from("./z.py")];
        let kept = restore_passed_paths(
            diagnostics.clone(),
            &files,
            Err(io::Error::other("no working directory")),
        );
        assert_eq!(kept, diagnostics);
    }
}
