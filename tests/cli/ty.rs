use std::fs;

use assert_cmd::Command;
use serde_json::json;

use crate::common::*;

#[test]
fn type_error_is_reported_in_the_types_section() {
    let path = temp_python_file("x: int = \"a\"\n");
    let output = gawain()
        .env("PATH", path_with_venv())
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[types]"), "stdout was: {stdout}");
    assert!(
        stdout.contains("ty[invalid-assignment]"),
        "stdout was: {stdout}"
    );
    assert!(!stdout.contains("[presence]"), "stdout was: {stdout}");
}

#[test]
fn clean_file_passes_both_phases() {
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", path_with_venv())
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[test]
fn presence_error_hides_the_types_section() {
    let path = temp_python_file("def f(a): ...\nx: int = \"a\"\n");
    let output = gawain()
        .env("PATH", path_with_venv())
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[presence]"), "stdout was: {stdout}");
    assert!(!stdout.contains("[types]"), "stdout was: {stdout}");
    assert!(!stdout.contains("ty["), "stdout was: {stdout}");
}

#[test]
fn json_output_has_presence_and_types_sections() {
    let path = temp_python_file("x: int = \"a\"\n");
    let output = gawain()
        .env("PATH", path_with_venv())
        .args(["check", "--format", "json"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.starts_with("{\n  \"checker\": \"ty\",\n  \"presence\": "),
        "stdout was: {text}"
    );
    let report = json_report(&output.stdout);
    assert_eq!(report["checker"], "ty");
    assert_eq!(report["presence"], json!([]));
    let types = report["types"].as_array().expect("types is an array");
    assert_eq!(types.len(), 1, "types were: {types:?}");
    assert_eq!(types[0]["rule"], "invalid-assignment");
    assert_eq!(types[0]["line"], 1);
    assert_eq!(types[0]["col"], 10);
    assert_eq!(types[0]["severity"], "error");
}

#[cfg(unix)]
#[test]
fn json_types_severity_is_error_or_warning() {
    let body = r#"printf '%s\n' '[{"check_name":"invalid-assignment","description":"invalid-assignment: bad","severity":"major","location":{"path":"x.py","positions":{"begin":{"line":1,"column":1}}}},{"check_name":"deprecated","description":"deprecated: old","severity":"minor","location":{"path":"x.py","positions":{"begin":{"line":2,"column":1}}}}]'; exit 1"#;
    let dir = fake_ty_dir(body);
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", path_only(&dir))
        .args(["check", "--format", "json"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    let report = json_report(&output.stdout);
    assert_eq!(report["types"][0]["severity"], "error");
    assert_eq!(report["types"][1]["severity"], "warning");
}

#[cfg(unix)]
#[test]
fn checker_flag_ty_runs_ty() {
    let dir = fake_ty_dir("echo '[]'");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", path_only(&dir))
        .args(["check", "--checker", "ty"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[cfg(unix)]
#[test]
fn checker_flag_overrides_the_config_checker() {
    let dir = temp_dir_with(
        "override-checker",
        &[
            ("pyproject.toml", b"[tool.gawain]\nchecker = \"pyrefly\"\n"),
            ("case.py", b"y: int = 1\n"),
        ],
    );
    let ty_dir = fake_ty_dir("echo '[]'");
    let output = gawain()
        .current_dir(&dir)
        .env("PATH", path_only(&ty_dir))
        .args(["check", "--checker", "ty", "case.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");
    fs::remove_dir_all(&ty_dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[test]
fn missing_ty_is_reported_with_the_fix() {
    let dir = unique_temp_path("no-ty");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", &dir)
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(TY_NOT_FOUND), "stderr was: {stderr}");
}

#[test]
fn no_types_works_without_ty() {
    let dir = unique_temp_path("no-ty");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", &dir)
        .args(["check", "--no-types"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[cfg(unix)]
#[test]
fn fake_ty_with_no_diagnostics_passes() {
    let output = run_with_fake_ty("echo '[]'", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence, types).\n"
    );
}

#[cfg(unix)]
#[test]
fn fake_ty_with_unparseable_output_fails_with_exit_2() {
    let output = run_with_fake_ty(
        "echo 'not json'; echo 'boom from ty' >&2; exit 1",
        "y: int = 1\n",
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: ty failed (exit 1): boom from ty"),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn fake_ty_with_an_unexpected_exit_code_fails_with_exit_2() {
    let output = run_with_fake_ty("exit 3", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: ty failed (exit 3)"),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn fake_ty_warning_is_shown_and_passes() {
    let body = format!("printf '%s\\n' '{MINOR_WARNING}'");
    let output = run_with_fake_ty(&body, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[types]"), "stdout was: {stdout}");
    assert!(
        stdout.contains("x.py:1:5  ty[deprecated] The function `f` is deprecated: use g"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("1 warning (types)."),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn ty_is_run_so_that_warnings_do_not_fail_it() {
    let args_file = unique_temp_path("ty-args.txt");
    let body = format!(
        "printf '%s\\n' \"$@\" > '{}'; echo '[]'",
        args_file.display()
    );
    let output = run_with_fake_ty(&body, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(0));
    let recorded = fs::read_to_string(&args_file).expect("fake ty recorded its arguments");
    fs::remove_file(&args_file).expect("argument file is removable");
    let args: Vec<&str> = recorded.lines().collect();
    assert_eq!(
        &args[..4],
        [
            "check",
            "--output-format",
            "gitlab",
            "--exit-zero-on-warning"
        ]
    );
}

/// `datetime.utcnow` is deprecated in typeshed, so ty 0.0.100 reports one `deprecated` warning for it
/// whatever the target Python version.
#[test]
fn ty_warning_alone_passes_with_exit_0() {
    let path = temp_python_file(concat!(
        "import datetime\n",
        "\n",
        "\n",
        "def now() -> datetime.datetime:\n",
        "    return datetime.datetime.utcnow()\n",
    ));
    let output = gawain()
        .env("PATH", path_with_venv())
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ty[deprecated]"), "stdout was: {stdout}");
    assert!(
        stdout.contains("1 warning (types)."),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn ty_failing_without_diagnostics_fails_the_types_phase() {
    let output = run_with_fake_ty("echo '[]'; exit 1", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ty failed without diagnostics (types)."),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn ty_next_to_gawain_is_used_before_path() {
    let beside = fake_ty_dir("echo '[]'");
    fs::copy(env!("CARGO_BIN_EXE_gawain"), beside.join("gawain")).expect("gawain is copyable");
    let empty = unique_temp_path("no-ty");
    fs::create_dir_all(&empty).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");

    let output = Command::new(beside.join("gawain"))
        .env("PATH", &empty)
        .arg("check")
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
fn two_type_errors_are_counted_in_plural() {
    let path = temp_python_file("x: int = \"a\"\ny: int = \"b\"\n");
    let output = gawain()
        .env("PATH", path_with_venv())
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 errors (types)."), "stdout was: {stdout}");
}

#[cfg(unix)]
#[test]
fn errors_and_warnings_are_counted_separately() {
    let body = r#"printf '%s\n' '[{"check_name":"invalid-assignment","description":"invalid-assignment: bad","severity":"major","location":{"path":"x.py","positions":{"begin":{"line":1,"column":1}}}},{"check_name":"deprecated","description":"deprecated: old","severity":"minor","location":{"path":"x.py","positions":{"begin":{"line":2,"column":1}}}}]'; exit 1"#;
    let output = run_with_fake_ty(body, "y: int = 1\n");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 error, 1 warning (types)."),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn ty_without_execute_permission_cannot_start() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fake_ty_dir("exit 0");
    fs::set_permissions(dir.join("ty"), fs::Permissions::from_mode(0o644))
        .expect("script permissions are settable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .env("PATH", path_only(&dir))
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: cannot run "),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn ty_killed_by_a_signal_fails_with_exit_2() {
    let output = run_with_fake_ty("kill -KILL $$", "y: int = 1\n");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: ty failed (exit signal):"),
        "stderr was: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn ty_gets_the_files_after_a_double_dash() {
    let args_file = unique_temp_path("ty-argv-dash.txt");
    let body = format!(
        "printf '%s\\n' \"$@\" > '{}'; echo '[]'",
        args_file.display()
    );
    let ty_dir = fake_ty_dir(&body);
    let dir = temp_tree("dash-file-argv", &[("-bad.py", "y: int = 1\n")]);
    let output = gawain()
        .current_dir(&dir)
        .env("PATH", path_only(&ty_dir))
        .args(["check", "--", "-bad.py"])
        .output()
        .expect("gawain runs");
    let recorded = fs::read_to_string(&args_file).expect("fake ty recorded its arguments");
    fs::remove_file(&args_file).expect("argument file is removable");
    fs::remove_dir_all(&ty_dir).expect("temp directory is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    let args: Vec<&str> = recorded.lines().collect();
    assert_eq!(
        args,
        [
            "check",
            "--output-format",
            "gitlab",
            "--exit-zero-on-warning",
            "--",
            "-bad.py"
        ]
    );
}

#[test]
fn file_whose_name_starts_with_a_dash_is_type_checked() {
    let dir = temp_tree("dash-file-ty", &[("-bad.py", "x: int = \"a\"\n")]);
    let output = gawain()
        .current_dir(&dir)
        .env("PATH", path_with_venv())
        .args(["check", "--", "-bad.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ty[invalid-assignment]"),
        "stdout was: {stdout}"
    );
}
