use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use serde_json::Value;

pub(crate) const FULLY_ANNOTATED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/fully_annotated.py"
);

pub(crate) const TY_NOT_FOUND: &str =
    "gawain: ty not found; install gawain[ty] (it depends on ty==0.0.100) or put ty on PATH";

pub(crate) const PYREFLY_NOT_FOUND: &str = "gawain: pyrefly not found; install gawain[pyrefly] (it depends on pyrefly==1.4.1) or put pyrefly on PATH";

/// One `minor` warning as ty 0.0.100 prints it in gitlab format.
#[cfg(unix)]
pub(crate) const MINOR_WARNING: &str = r#"[{"check_name":"deprecated","description":"deprecated: The function `f` is deprecated: use g","severity":"minor","fingerprint":"901c3ffc76ecb125","location":{"path":"x.py","positions":{"begin":{"line":1,"column":5},"end":{"line":1,"column":6}}}}]"#;

pub(crate) fn gawain() -> Command {
    Command::cargo_bin("gawain").expect("the gawain binary is built for integration tests")
}

/// A path in the system temp directory that no other test run uses, ending in `name`.
pub(crate) fn unique_temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "gawain-cli-test-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// Writes `contents` to a uniquely named `.py` file in the system temp directory.
pub(crate) fn temp_python_file(contents: &str) -> PathBuf {
    let path = unique_temp_path("case.py");
    fs::write(&path, contents).expect("temp file is writable");
    path
}

/// The project venv's bin directory, where `uv pip install --python .venv ty==0.0.100` puts ty.
pub(crate) fn venv_bin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv/bin")
}

/// PATH with the project venv first, so gawain finds ty 0.0.100 there before any other ty.
pub(crate) fn path_with_venv() -> OsString {
    let bin = venv_bin();
    assert!(
        bin.join("ty").is_file(),
        "no ty in {}: run `uv pip install --python .venv ty==0.0.100`",
        bin.display()
    );
    path_with_first(bin)
}

/// PATH with the project venv first, so gawain finds pyrefly 1.4.1 there before any other pyrefly.
pub(crate) fn path_with_venv_pyrefly() -> OsString {
    let bin = venv_bin();
    assert!(
        bin.join("pyrefly").is_file(),
        "no pyrefly in {}: run `uv pip install --python .venv pyrefly==1.4.1`",
        bin.display()
    );
    path_with_first(bin)
}

/// This process's PATH with `dir` in front of it.
pub(crate) fn path_with_first(dir: PathBuf) -> OsString {
    let mut dirs = vec![dir];
    if let Some(current) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(dirs).expect("PATH entries hold no separator")
}

/// A directory holding an executable `ty` shell script whose commands are `body`.
#[cfg(unix)]
pub(crate) fn fake_ty_dir(body: &str) -> PathBuf {
    fake_binary_dir("ty", body)
}

/// A directory holding an executable `pyrefly` shell script whose commands are `body`.
#[cfg(unix)]
pub(crate) fn fake_pyrefly_dir(body: &str) -> PathBuf {
    fake_binary_dir("pyrefly", body)
}

/// A directory holding an executable shell script called `name` whose commands are `body`.
#[cfg(unix)]
pub(crate) fn fake_binary_dir(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = unique_temp_path(&format!("fake-{name}"));
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let script = dir.join(name);
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("temp file is writable");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("script is executable");
    dir
}

/// A PATH that holds `dir` and the system directories, so the shell of a fake ty runs.
#[cfg(unix)]
pub(crate) fn path_only(dir: &Path) -> OsString {
    let dirs = [
        dir.to_path_buf(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    std::env::join_paths(dirs).expect("PATH entries hold no separator")
}

/// Runs gawain on one file holding `contents`, with a fake ty whose script runs `body`.
#[cfg(unix)]
pub(crate) fn run_with_fake_ty(body: &str, contents: &str) -> Output {
    let dir = fake_ty_dir(body);
    let path = temp_python_file(contents);
    let output = gawain()
        .env("PATH", path_only(&dir))
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");
    output
}

/// One item of pyrefly's JSON output, at line 1, column 10 of `x.py`, with the given rule `name` and
/// `severity`. Its text has no single quote, so a fake pyrefly can echo it.
#[cfg(unix)]
pub(crate) fn pyrefly_one_item(name: &str, severity: &str) -> String {
    pyrefly_item_at("x.py", name, severity)
}

/// One item of pyrefly's JSON output, at line 1, column 10 of the file that pyrefly names `path`.
#[cfg(unix)]
pub(crate) fn pyrefly_item_at(path: &str, name: &str, severity: &str) -> String {
    format!(
        r#"{{"errors": [{{"line": 1, "column": 10, "stop_line": 1, "stop_column": 13, "path": "{path}", "code": -2, "name": "{name}", "description": "`str` is not assignable to `int`", "concise_description": "`str` is not assignable to `int`", "severity": "{severity}"}}]}}"#
    )
}

/// Runs gawain with `--checker pyrefly` on one file holding `contents`, with a fake pyrefly whose script
/// runs `body`.
#[cfg(unix)]
pub(crate) fn run_with_fake_pyrefly(body: &str, contents: &str) -> Output {
    let dir = fake_pyrefly_dir(body);
    let path = temp_python_file(contents);
    let output = gawain()
        .env("PATH", path_only(&dir))
        .args(["check", "--checker", "pyrefly"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");
    output
}

/// The arguments that gawain passes to pyrefly when it checks `file`. A fake pyrefly records them, one per
/// line, and prints an empty report.
#[cfg(unix)]
pub(crate) fn pyrefly_args_for(file: &Path) -> Vec<String> {
    recorded_pyrefly_args(None, file)
}

/// The arguments that gawain passes to pyrefly when it checks `file` from the working directory `cwd`, or from
/// the test's own working directory when `cwd` is `None`. The fake pyrefly records them, one per line.
#[cfg(unix)]
pub(crate) fn recorded_pyrefly_args(cwd: Option<&Path>, file: &Path) -> Vec<String> {
    let args_file = unique_temp_path("pyrefly-args.txt");
    let body = format!(
        r#"printf '%s\n' "$@" > '{}'; echo '{{"errors": []}}'"#,
        args_file.display()
    );
    let dir = fake_pyrefly_dir(&body);
    let mut command = gawain();
    command
        .env("PATH", path_only(&dir))
        .args(["check", "--checker", "pyrefly"]);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.arg(file).output().expect("gawain runs");
    let recorded = fs::read_to_string(&args_file).expect("fake pyrefly recorded its arguments");
    fs::remove_file(&args_file).expect("argument file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    recorded.lines().map(str::to_owned).collect()
}

/// Parses gawain's JSON output, which is one object with `presence` and `types`.
pub(crate) fn json_report(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is a JSON object")
}

/// Creates a uniquely named directory that holds `files`, each given as its name and its bytes.
pub(crate) fn temp_dir_with(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = unique_temp_path(name);
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    for (file, bytes) in files {
        fs::write(dir.join(file), bytes).expect("temp file is writable");
    }
    dir
}

/// Each presence diagnostic of gawain's JSON `report` as `(file name, line, col, code, message)`.
pub(crate) fn presence_rows(report: &Value) -> Vec<(&str, u64, u64, &str, &str)> {
    report["presence"]
        .as_array()
        .expect("presence is an array")
        .iter()
        .map(|diagnostic| {
            let file = diagnostic["path"].as_str().expect("path is a string");
            (
                Path::new(file)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("path ends in a file name"),
                diagnostic["line"].as_u64().expect("line is a number"),
                diagnostic["col"].as_u64().expect("col is a number"),
                diagnostic["code"].as_str().expect("code is a string"),
                diagnostic["message"].as_str().expect("message is a string"),
            )
        })
        .collect()
}

/// Creates a uniquely named directory holding `files`, each given as its path relative to the directory
/// and its text. Parent directories are created as needed.
pub(crate) fn temp_tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = unique_temp_path(name);
    for (file, text) in files {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().expect("file has a parent directory"))
            .expect("temp directory is creatable");
        fs::write(&path, text).expect("temp file is writable");
    }
    dir
}

/// Runs the presence phase, `gawain check --no-types --format json`, on `path`. Returns the exit code and
/// the parsed report.
pub(crate) fn check_json(path: &Path) -> (Option<i32>, Value) {
    let output = gawain()
        .args(["check", "--no-types", "--format", "json"])
        .arg(path)
        .output()
        .expect("gawain runs");
    (output.status.code(), json_report(&output.stdout))
}

/// Runs the presence phase like `check_json`, in strict mode, for a test whose diagnostics come from a
/// first binding of an obvious value.
pub(crate) fn check_json_strict(path: &Path) -> (Option<i32>, Value) {
    let output = gawain()
        .args([
            "check",
            "--no-types",
            "--format",
            "json",
            "--mode",
            "strict",
        ])
        .arg(path)
        .output()
        .expect("gawain runs");
    (output.status.code(), json_report(&output.stdout))
}

/// Runs the presence phase only, on one file holding `contents`, and returns the output.
pub(crate) fn check_presence_of(contents: &str) -> Output {
    let path = temp_python_file(contents);
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    output
}

/// Runs `gawain check --no-types <args> case.py` with `dir` as the working directory, so that the config of
/// `dir` is the one gawain reads.
pub(crate) fn check_case_in(dir: &Path, args: &[&str]) -> Output {
    gawain()
        .current_dir(dir)
        .args(["check", "--no-types"])
        .args(args)
        .arg("case.py")
        .output()
        .expect("gawain runs")
}
