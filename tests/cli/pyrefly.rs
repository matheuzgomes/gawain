use std::fs;
use std::path::Path;

use assert_cmd::Command;

use crate::common::*;

#[cfg(unix)]
#[test]
fn fake_pyrefly_with_no_errors_passes() {
    let output = run_with_fake_pyrefly(r#"echo '{"errors": []}'"#, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[cfg(unix)]
#[test]
fn fake_pyrefly_error_fails_the_types_phase() {
    let body = format!(
        "echo '{}'; exit 1",
        pyrefly_one_item("bad-assignment", "error")
    );
    let output = run_with_fake_pyrefly(&body, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("x.py:1:10  pyrefly[bad-assignment] `str` is not assignable to `int`"),
        "stdout was: {stdout}"
    );
    assert!(stdout.contains("1 error (types)."), "stdout was: {stdout}");
}

#[cfg(unix)]
#[test]
fn fake_pyrefly_warning_is_shown_and_passes() {
    let body = format!("echo '{}'; exit 1", pyrefly_one_item("deprecated", "warn"));
    let output = run_with_fake_pyrefly(&body, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("x.py:1:10  pyrefly[deprecated] `str` is not assignable to `int`"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("1 warning (types)."),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn fake_pyrefly_with_an_unexpected_exit_code_fails_with_exit_2() {
    let output = run_with_fake_pyrefly("exit 3", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: pyrefly failed (exit 3)"),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn fake_pyrefly_with_output_that_is_not_json_fails_with_exit_2() {
    let output = run_with_fake_pyrefly("echo 'not json'", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: pyrefly failed (exit 0)"),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn pyrefly_gets_json_flags_and_the_file_after_a_double_dash() {
    // The temp directory holds no pyrefly config, so gawain adds `--preset default`.
    let path = temp_python_file("y: int = 1\n");
    let args = pyrefly_args_for(&path);
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(
        args,
        [
            "check",
            "--output-format",
            "json",
            "--summary=none",
            "--min-severity",
            "warn",
            "--preset",
            "default",
            "--",
            path.to_str().expect("temp path is UTF-8"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn pyrefly_toml_beside_the_file_means_no_preset_flag() {
    let dir = temp_dir_with(
        "pyrefly-toml-beside",
        &[("pyrefly.toml", b""), ("case.py", b"y: int = 1\n")],
    );
    let path = dir.join("case.py");
    let args = pyrefly_args_for(&path);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(
        args,
        [
            "check",
            "--output-format",
            "json",
            "--summary=none",
            "--min-severity",
            "warn",
            "--",
            path.to_str().expect("temp path is UTF-8"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn pyproject_with_a_pyrefly_table_means_no_preset_flag() {
    let dir = temp_dir_with(
        "pyrefly-table-beside",
        &[
            ("pyproject.toml", b"[tool.pyrefly]\n"),
            ("case.py", b"y: int = 1\n"),
        ],
    );
    let path = dir.join("case.py");
    let args = pyrefly_args_for(&path);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(
        args,
        [
            "check",
            "--output-format",
            "json",
            "--summary=none",
            "--min-severity",
            "warn",
            "--",
            path.to_str().expect("temp path is UTF-8"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn pyrefly_config_in_the_working_directory_does_not_cover_a_sibling_directory() {
    let root = temp_tree(
        "sibling-config",
        &[
            ("app/pyproject.toml", "[tool.pyrefly]\n"),
            ("other/x.py", "y: int = 1\n"),
        ],
    );
    let args = recorded_pyrefly_args(Some(&root.join("app")), Path::new("../other/x.py"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(
        args,
        [
            "check",
            "--output-format",
            "json",
            "--summary=none",
            "--min-severity",
            "warn",
            "--preset",
            "default",
            "--",
            "../other/x.py",
        ]
    );
}

#[cfg(unix)]
#[test]
fn pyrefly_path_is_mapped_back_to_the_absolute_file_that_was_passed() {
    let root = fs::canonicalize(temp_tree(
        "absolute-passed",
        &[("other/x.py", "y: int = 1\n")],
    ))
    .expect("temp directory is canonical");
    let app = root.join("app");
    fs::create_dir_all(&app).expect("temp directory is creatable");
    let file = root.join("other/x.py");
    // pyrefly prints the file relative to its working directory, which is `app`.
    let body = format!(
        "echo '{}'; exit 1",
        pyrefly_item_at("../other/x.py", "bad-assignment", "error")
    );
    let dir = fake_pyrefly_dir(&body);
    let output = gawain()
        .current_dir(&app)
        .env("PATH", path_only(&dir))
        .args(["check", "--checker", "pyrefly"])
        .arg(&file)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&root).expect("temp directory is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = format!("{}:1:10  pyrefly[bad-assignment]", file.display());
    assert!(
        stdout.lines().any(|line| line.starts_with(&row)),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn pyrefly_next_to_gawain_is_used_before_path() {
    let beside = fake_pyrefly_dir(r#"echo '{"errors": []}'"#);
    fs::copy(env!("CARGO_BIN_EXE_gawain"), beside.join("gawain")).expect("gawain is copyable");
    let empty = unique_temp_path("no-pyrefly");
    fs::create_dir_all(&empty).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");

    let output = Command::new(beside.join("gawain"))
        .env("PATH", &empty)
        .args(["check", "--checker", "pyrefly"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&beside).expect("temp directory is removable");
    fs::remove_dir_all(&empty).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[test]
fn real_pyrefly_reports_a_bad_return() {
    let path = temp_python_file("def total(prices: list[int]) -> int:\n    return \"0\"\n");
    let output = gawain()
        .env("PATH", path_with_venv_pyrefly())
        .args(["check", "--checker", "pyrefly"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pyrefly[bad-return]"),
        "stdout was: {stdout}"
    );
}

#[test]
fn real_pyrefly_reports_a_file_in_a_sibling_directory_under_the_path_passed() {
    // The working directory holds a pyrefly config, so the walk from `../b` must not take it for `b`'s config.
    let root = temp_tree(
        "real-sibling",
        &[
            ("a/pyproject.toml", "[tool.pyrefly]\n"),
            ("b/bad.py", "x: int = \"a\"\n"),
        ],
    );
    let app = root.join("a");
    let output = gawain()
        .current_dir(&app)
        .env("PATH", path_with_venv_pyrefly())
        .args(["check", "--checker", "pyrefly", "../b/bad.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("../b/bad.py:1:10  pyrefly[bad-assignment]"),
        "stdout was: {stdout}"
    );
}
