use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use serde_json::{Value, json};

const FULLY_ANNOTATED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/fully_annotated.py"
);
const TY_NOT_FOUND: &str = "gawain: ty not found; install gawain with pip/uv (it depends on ty==0.0.100) or put ty on PATH";
/// One `minor` warning as ty 0.0.100 prints it in gitlab format.
#[cfg(unix)]
const MINOR_WARNING: &str = r#"[{"check_name":"deprecated","description":"deprecated: The function `f` is deprecated: use g","severity":"minor","fingerprint":"901c3ffc76ecb125","location":{"path":"x.py","positions":{"begin":{"line":1,"column":5},"end":{"line":1,"column":6}}}}]"#;

fn gawain() -> Command {
    Command::cargo_bin("gawain").expect("the gawain binary is built for integration tests")
}

/// A path in the system temp directory that no other test run uses, ending in `name`.
fn unique_temp_path(name: &str) -> PathBuf {
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
fn temp_python_file(contents: &str) -> PathBuf {
    let path = unique_temp_path("case.py");
    fs::write(&path, contents).expect("temp file is writable");
    path
}

/// The project venv's bin directory, where `uv pip install --python .venv ty==0.0.100` puts ty.
fn venv_bin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv/bin")
}

/// PATH with the project venv first, so gawain finds ty 0.0.100 there before any other ty.
fn path_with_venv() -> OsString {
    let bin = venv_bin();
    assert!(
        bin.join("ty").is_file(),
        "no ty in {}: run `uv pip install --python .venv ty==0.0.100`",
        bin.display()
    );
    let mut dirs = vec![bin];
    if let Some(current) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(dirs).expect("PATH entries hold no separator")
}

/// A directory holding an executable `ty` shell script whose commands are `body`.
#[cfg(unix)]
fn fake_ty_dir(body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = unique_temp_path("fake-ty");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let script = dir.join("ty");
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("temp file is writable");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("script is executable");
    dir
}

/// A PATH that holds `dir` and the system directories, so the shell of a fake ty runs.
#[cfg(unix)]
fn path_only(dir: &Path) -> OsString {
    let dirs = [
        dir.to_path_buf(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    std::env::join_paths(dirs).expect("PATH entries hold no separator")
}

/// Runs gawain on one file holding `contents`, with a fake ty whose script runs `body`.
#[cfg(unix)]
fn run_with_fake_ty(body: &str, contents: &str) -> Output {
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

/// Parses gawain's JSON output, which is one object with `presence` and `types`.
fn json_report(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("stdout is a JSON object")
}

#[test]
fn check_without_paths_is_a_usage_error() {
    gawain().arg("check").assert().code(2);
}

/// Creates a uniquely named directory that holds `files`, each given as its name and its bytes.
fn temp_dir_with(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = unique_temp_path(name);
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    for (file, bytes) in files {
        fs::write(dir.join(file), bytes).expect("temp file is writable");
    }
    dir
}

/// Each presence diagnostic of gawain's JSON `report` as `(file name, line, col, code, message)`.
fn presence_rows(report: &Value) -> Vec<(&str, u64, u64, &str, &str)> {
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

#[test]
fn syntax_error_exits_1_as_a_presence_error() {
    let path = temp_python_file("def broken(:\n    pass\n");
    let output = gawain()
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty(), "stderr was: {:?}", output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[presence]"), "stdout was: {stdout}");
    assert!(
        stdout
            .contains("GW000 syntax error: Expected a parameter or the end of the parameter list"),
        "stdout was: {stdout}"
    );
    assert!(!stdout.contains("[types]"), "stdout was: {stdout}");
}

#[test]
fn syntax_error_is_gw000_and_a_gw001_in_another_file_is_still_reported() {
    let dir = temp_dir_with(
        "syntax-and-gw001",
        &[
            ("broken.py", b"def broken(:\n    pass\n"),
            ("flagged.py", b"def f() -> None:\n    value = 1\n"),
        ],
    );
    let output = gawain()
        .args(["check", "--format", "json", "--mode", "strict"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [
            (
                "broken.py",
                1,
                12,
                "GW000",
                "syntax error: Expected a parameter or the end of the parameter list"
            ),
            (
                "broken.py",
                1,
                13,
                "GW000",
                "syntax error: Expected `)`, found newline"
            ),
            (
                "flagged.py",
                2,
                5,
                "GW001",
                "variable 'value' has no type annotation"
            ),
        ]
    );
}

#[test]
fn two_syntax_errors_in_one_file_are_two_gw000() {
    let dir = temp_dir_with("two-errors", &[("two.py", b"x = 1 +\ny = 2 +\n")]);
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [
            (
                "two.py",
                1,
                8,
                "GW000",
                "syntax error: Expected an expression"
            ),
            (
                "two.py",
                2,
                8,
                "GW000",
                "syntax error: Expected an expression"
            ),
        ]
    );
}

#[test]
fn non_utf8_file_is_gw000_and_the_other_files_are_still_checked() {
    let dir = temp_dir_with(
        "latin1",
        &[
            ("latin1.py", b"x = '\xe9'\n"),
            ("flagged.py", b"def f(a): ...\n"),
        ],
    );
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [
            (
                "flagged.py",
                1,
                5,
                "GW003",
                "function 'f' has no return type annotation"
            ),
            (
                "flagged.py",
                1,
                7,
                "GW002",
                "parameter 'a' of function 'f' has no type annotation"
            ),
            ("latin1.py", 1, 1, "GW000", "file is not valid UTF-8"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_gw000_and_the_other_files_are_still_checked() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir_with(
        "unreadable",
        &[
            ("locked.py", b"x = 1\n"),
            ("flagged.py", b"def f(a): ...\n"),
        ],
    );
    let locked = dir.join("locked.py");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
        .expect("permissions are settable");
    if fs::read(&locked).is_ok() {
        // Root reads any file, so there is no unreadable file to check.
        fs::remove_dir_all(&dir).expect("temp directory is removable");
        return;
    }
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [
            (
                "flagged.py",
                1,
                5,
                "GW003",
                "function 'f' has no return type annotation"
            ),
            (
                "flagged.py",
                1,
                7,
                "GW002",
                "parameter 'a' of function 'f' has no type annotation"
            ),
            (
                "locked.py",
                1,
                1,
                "GW000",
                "cannot read file: Permission denied (os error 13)"
            ),
        ]
    );
}

#[test]
fn json_output_reports_a_syntax_error_with_the_code_gw000() {
    let path = temp_python_file("def broken(:\n    pass\n");
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"code\": \"GW000\""),
        "stdout was: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn ty_does_not_run_while_a_file_has_a_syntax_error() {
    let marker = unique_temp_path("ty-ran.txt");
    let body = format!("touch '{}'; echo '[]'", marker.display());
    let ty_dir = fake_ty_dir(&body);
    let clean = temp_dir_with("ty-clean", &[("clean.py", b"y: int = 1\n")]);
    let mixed = temp_dir_with(
        "ty-skipped",
        &[
            ("clean.py", b"y: int = 1\n"),
            ("broken.py", b"def broken(:\n    pass\n"),
        ],
    );

    let control = gawain()
        .env("PATH", path_only(&ty_dir))
        .arg("check")
        .arg(&clean)
        .output()
        .expect("gawain runs");
    let ty_ran_on_clean = marker.exists();
    let _ = fs::remove_file(&marker);

    let output = gawain()
        .env("PATH", path_only(&ty_dir))
        .arg("check")
        .arg(&mixed)
        .output()
        .expect("gawain runs");
    let ty_ran_on_mixed = marker.exists();

    let _ = fs::remove_file(&marker);
    fs::remove_dir_all(&clean).expect("temp directory is removable");
    fs::remove_dir_all(&mixed).expect("temp directory is removable");
    fs::remove_dir_all(&ty_dir).expect("temp directory is removable");

    assert_eq!(control.status.code(), Some(0));
    assert!(ty_ran_on_clean, "the fake ty did not run on a clean run");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GW000 syntax error"),
        "stdout was: {stdout}"
    );
    assert!(!ty_ran_on_mixed, "ty ran while a file had a syntax error");
}

#[test]
fn gw000_cannot_be_suppressed_and_listing_it_is_an_unknown_code() {
    let dir = temp_dir_with(
        "gw000-ignore",
        &[("ok.py", b"x: int = 1  # gawain: ignore[GW000]\n")],
    );
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [(
            "ok.py",
            1,
            13,
            "GW900",
            "unknown code 'GW000' in 'gawain: ignore'"
        )]
    );
}

#[test]
fn a_suppression_comment_does_not_silence_gw000() {
    let dir = temp_dir_with(
        "gw000-broken-ignore",
        &[(
            "broken.py",
            b"def broken(:  # gawain: ignore[GW000]\n    pass\n",
        )],
    );
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    let rows = presence_rows(&report);
    assert!(!rows.is_empty(), "no diagnostics were reported");
    assert!(
        rows.iter().all(|row| row.3 == "GW000" && row.1 == 1),
        "rows were: {rows:?}"
    );
}

#[test]
fn invalid_file_is_left_out_of_the_class_index() {
    // `Base` declares `y` and so would be indexed if bad.py parsed. Left out, it is an external base,
    // and the attribute rule does not report `self.x` in child.py.
    let dir = temp_dir_with(
        "invalid-base",
        &[
            (
                "bad.py",
                b"class Base:\n    y: int = 0\n\ndef broken(:\n    pass\n",
            ),
            (
                "child.py",
                b"from bad import Base\n\nclass Child(Base):\n    def set(self) -> None:\n        self.x = 1\n",
            ),
        ],
    );
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    let rows = presence_rows(&report);
    assert!(!rows.is_empty(), "no diagnostics were reported");
    assert!(
        rows.iter().all(|row| row.0 == "bad.py" && row.3 == "GW000"),
        "rows were: {rows:?}"
    );
}

#[test]
fn missing_annotations_are_reported_in_text_output() {
    let path = temp_python_file("def f(a): ...\n");
    let output = gawain()
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GW002 parameter 'a' of function 'f' has no type annotation"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("GW003 function 'f' has no return type annotation"),
        "stdout was: {stdout}"
    );
}

#[test]
fn diagnostics_point_at_the_parameter_name_and_the_function_name() {
    let path = temp_python_file("def f(a): ...\n");
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    let report = json_report(&output.stdout);
    let located: Vec<(&str, u64, u64)> = report["presence"]
        .as_array()
        .expect("presence is an array")
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().expect("code is a string"),
                diagnostic["line"].as_u64().expect("line is a number"),
                diagnostic["col"].as_u64().expect("col is a number"),
            )
        })
        .collect();
    assert_eq!(located, [("GW003", 1, 5), ("GW002", 1, 7)]);
}

#[test]
fn directory_walk_checks_python_files_only() {
    let dir = unique_temp_path("dir");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    fs::write(dir.join("module.py"), "def f(a): ...\n").expect("temp file is writable");
    fs::write(dir.join("notes.txt"), "def g(b): ...\n").expect("temp file is writable");

    let output = gawain()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("function 'f'"), "stdout was: {stdout}");
    assert!(!stdout.contains("'g'"), "stdout was: {stdout}");
}

#[test]
fn directory_walk_skips_the_exclude_list_from_pyproject() {
    // `flagged.py` would be reported if it were checked, so exit 0 shows that the walk skipped it.
    let dir = temp_tree(
        "walk-skips-excluded",
        &[
            ("pyproject.toml", "[tool.gawain]\nexclude = [\"skipped\"]\n"),
            ("skipped/flagged.py", "def f(a): ...\n"),
            ("kept.py", "y: int = 1\n"),
        ],
    );
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "--no-types", "."])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[test]
fn fully_annotated_fixture_passes() {
    gawain()
        .args(["check", "--no-types", FULLY_ANNOTATED])
        .assert()
        .code(0)
        .stdout("All checks passed (presence).\n");
}

#[test]
fn variable_diagnostic_points_at_the_target_name() {
    let path = temp_python_file("def f() -> None:\n    value = 1\n");
    let output = gawain()
        .args(["check", "--format", "json", "--mode", "strict"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    let report = json_report(&output.stdout);
    let diagnostics = report["presence"].as_array().expect("presence is an array");
    assert_eq!(diagnostics.len(), 1, "diagnostics were: {diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], "GW001");
    assert_eq!(diagnostic["line"], 2);
    assert_eq!(diagnostic["col"], 5);
    assert_eq!(
        diagnostic["message"],
        "variable 'value' has no type annotation"
    );
}

#[test]
fn malformed_suppression_is_reported_as_gw900_in_text_output() {
    let path = temp_python_file("c = 1  # gawain: ignore\n");
    let output = gawain()
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = format!(
        "{}:1:8  GW900 'gawain: ignore' needs at least one code, e.g. '# gawain: ignore[GW001]'",
        path.display()
    );
    assert!(
        stdout.lines().any(|line| line == expected),
        "stdout was: {stdout}"
    );
}

#[test]
fn unused_suppression_is_a_presence_error_that_skips_types() {
    let path = temp_python_file("x: int = \"a\"  # gawain: ignore[GW001]\n");
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
    assert!(
        stdout.contains("GW901 unused suppression 'GW001'"),
        "stdout was: {stdout}"
    );
    assert!(!stdout.contains("[types]"), "stdout was: {stdout}");
}

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
    let report = json_report(&output.stdout);
    assert_eq!(report["presence"], json!([]));
    let types = report["types"].as_array().expect("types is an array");
    assert_eq!(types.len(), 1, "types were: {types:?}");
    assert_eq!(types[0]["rule"], "invalid-assignment");
    assert_eq!(types[0]["line"], 1);
    assert_eq!(types[0]["col"], 10);
    assert_eq!(types[0]["severity"], "major");
}

#[test]
fn json_types_is_null_with_no_types() {
    let output = gawain()
        .args(["check", "--no-types", "--format", "json", FULLY_ANNOTATED])
        .output()
        .expect("gawain runs");

    assert_eq!(output.status.code(), Some(0));
    let report = json_report(&output.stdout);
    assert_eq!(report["presence"], json!([]));
    assert_eq!(report["types"], Value::Null);
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

// macOS (APFS) refuses file names that are not UTF-8, so such a path cannot reach gawain there.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn json_for_a_path_that_is_not_utf8_is_an_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = unique_temp_path("non-utf8");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let path = dir.join(OsStr::from_bytes(b"bad\xff.py"));
    fs::write(&path, "def f(a): ...\n").expect("temp file is writable");
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: cannot render JSON"),
        "stderr was: {stderr}"
    );
}

#[test]
fn missing_path_is_a_usage_error() {
    let missing = unique_temp_path("missing.py");
    let output = gawain()
        .arg("check")
        .arg(&missing)
        .output()
        .expect("gawain runs");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: no such file or directory:"),
        "stderr was: {stderr}"
    );
}

#[test]
fn invalid_pyproject_in_the_working_directory_is_a_usage_error() {
    let dir = unique_temp_path("bad-config");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    fs::write(dir.join("pyproject.toml"), "[tool.gawain\n").expect("temp file is writable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .current_dir(&dir)
        .arg("check")
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gawain: pyproject.toml:"),
        "stderr was: {stderr}"
    );
}

#[test]
fn single_presence_error_is_named_in_the_singular() {
    let path = temp_python_file("def f() -> None:\n    value = 1\n");
    let output = gawain()
        .args(["check", "--mode", "strict"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 error (presence). Type check skipped until presence errors are fixed."),
        "stdout was: {stdout}"
    );
}

/// Creates a uniquely named directory holding `files`, each given as its path relative to the directory
/// and its text. Parent directories are created as needed.
fn temp_tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = unique_temp_path(name);
    for (file, text) in files {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().expect("file has a parent directory"))
            .expect("temp directory is creatable");
        fs::write(&path, text).expect("temp file is writable");
    }
    dir
}

#[test]
fn file_named_directly_without_a_python_extension_is_not_checked() {
    let dir = temp_tree("named-txt", &[("notes.txt", "def f(a): ...\n")]);
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(dir.join("notes.txt"))
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim_end(), "All checks passed (presence).");
}

#[cfg(unix)]
#[test]
fn symlink_to_a_python_file_is_checked_under_the_link_path() {
    let root = temp_tree(
        "link-to-file",
        &[
            ("outside/real.py", "def f(a): ...\n"),
            ("proj/keep.py", "y: int = 1\n"),
        ],
    );
    std::os::unix::fs::symlink(root.join("outside/real.py"), root.join("proj/linked.py"))
        .expect("link is creatable");
    let (code, report) = check_json(&root.join("proj"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [
            (
                "linked.py",
                1,
                5,
                "GW003",
                "function 'f' has no return type annotation"
            ),
            (
                "linked.py",
                1,
                7,
                "GW002",
                "parameter 'a' of function 'f' has no type annotation"
            ),
        ]
    );
    assert_eq!(
        Path::new(
            report["presence"][0]["path"]
                .as_str()
                .expect("path is a string")
        ),
        root.join("proj/linked.py")
    );
}

#[cfg(unix)]
#[test]
fn symlink_to_a_directory_is_not_descended() {
    let root = temp_tree(
        "link-to-dir",
        &[
            ("other/inside.py", "def f(a): ...\n"),
            ("proj/keep.py", "y: int = 1\n"),
        ],
    );
    std::os::unix::fs::symlink(root.join("other"), root.join("proj/linked_dir"))
        .expect("link is creatable");
    std::os::unix::fs::symlink(root.join("other"), root.join("proj/as_dir.py"))
        .expect("link is creatable");
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(root.join("proj"))
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[cfg(unix)]
#[test]
fn broken_symlink_named_py_is_gw000_and_the_run_fails() {
    let root = temp_tree("broken-link", &[("proj/keep.py", "y: int = 1\n")]);
    std::os::unix::fs::symlink(root.join("missing.py"), root.join("proj/broken.py"))
        .expect("link is creatable");
    let (code, report) = check_json(&root.join("proj"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [(
            "broken.py",
            1,
            1,
            "GW000",
            "cannot read file: No such file or directory (os error 2)"
        )]
    );
}

#[cfg(unix)]
#[test]
fn file_and_a_link_to_it_are_checked_once_under_the_file_itself() {
    let root = temp_tree("real-and-link", &[("proj/real.py", "def f(a): ...\n")]);
    std::os::unix::fs::symlink(root.join("proj/real.py"), root.join("proj/alias.py"))
        .expect("link is creatable");
    let (code, report) = check_json(&root.join("proj"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [
            (
                "real.py",
                1,
                5,
                "GW003",
                "function 'f' has no return type annotation"
            ),
            (
                "real.py",
                1,
                7,
                "GW002",
                "parameter 'a' of function 'f' has no type annotation"
            ),
        ]
    );
    assert_eq!(
        Path::new(
            report["presence"][0]["path"]
                .as_str()
                .expect("path is a string")
        ),
        root.join("proj/real.py")
    );
}

#[cfg(unix)]
#[test]
fn link_that_sorts_before_its_file_does_not_hide_the_file_from_imports() {
    let root = temp_tree(
        "link-sorts-first",
        &[
            ("proj/real.py", "class Base:\n    a: int = 0\nz = 1\n"),
            (
                "proj/use.py",
                "from real import Base\n\n\nclass K(Base):\n    def __init__(self) -> None:\n        self.q = 1\n",
            ),
        ],
    );
    std::os::unix::fs::symlink(root.join("proj/real.py"), root.join("proj/alias.py"))
        .expect("link is creatable");
    let (code, report) = check_json_strict(&root.join("proj"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [
            (
                "real.py",
                3,
                1,
                "GW001",
                "variable 'z' has no type annotation"
            ),
            (
                "use.py",
                6,
                14,
                "GW004",
                "attribute 'q' is not declared in the body of class 'K'"
            ),
        ]
    );
    assert_eq!(
        Path::new(
            report["presence"][0]["path"]
                .as_str()
                .expect("path is a string")
        ),
        root.join("proj/real.py")
    );
}

#[cfg(unix)]
#[test]
fn link_to_a_file_outside_its_package_is_named_by_the_link_for_imports() {
    let root = temp_tree(
        "link-module-name",
        &[
            ("outside/base.py", "class Base:\n    pass\n"),
            ("proj/app/__init__.py", ""),
            (
                "proj/app/user.py",
                "from app.linked import Base\n\n\nclass Sub(Base):\n    def __init__(self) -> None:\n        self.value = 1\n",
            ),
        ],
    );
    std::os::unix::fs::symlink(
        root.join("outside/base.py"),
        root.join("proj/app/linked.py"),
    )
    .expect("link is creatable");
    let (code, report) = check_json(&root.join("proj"));
    fs::remove_dir_all(&root).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [(
            "user.py",
            6,
            14,
            "GW004",
            "attribute 'value' is not declared in the body of class 'Sub'"
        )]
    );
}

#[test]
fn working_directory_without_a_pyproject_uses_the_default_config() {
    let dir = unique_temp_path("no-config");
    fs::create_dir_all(&dir).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "--no-types"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim_end(), "All checks passed (presence).");
}

#[test]
fn pyproject_that_cannot_be_read_is_a_usage_error() {
    let dir = unique_temp_path("config-is-a-directory");
    fs::create_dir_all(dir.join("pyproject.toml")).expect("temp directory is creatable");
    let path = temp_python_file("y: int = 1\n");
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "--no-types"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("gawain: pyproject.toml: "),
        "stderr was: {stderr}"
    );
}

#[test]
fn relative_import_above_the_top_package_binds_nothing() {
    let dir = temp_tree(
        "relative-above-top",
        &[
            ("b.py", "class Base:\n    pass\n"),
            (
                "case.py",
                "from .b import Base\n\n\nclass C(Base):\n    def __init__(self) -> None:\n        self.value = 1\n",
            ),
        ],
    );
    let (code, report) = check_json(&dir);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(code, Some(0));
    assert_eq!(presence_rows(&report), []);
}

#[test]
fn relative_import_of_two_levels_reads_the_class_of_the_parent_package() {
    let dir = temp_tree(
        "relative-two-levels",
        &[
            ("pkg/__init__.py", "class Base:\n    value: int = 0\n"),
            ("pkg/sub/__init__.py", ""),
            (
                "pkg/sub/mod.py",
                "from .. import Base\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        self.value = 1\n        self.other = 2\n",
            ),
        ],
    );
    let (code, report) = check_json(&dir);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [(
            "mod.py",
            7,
            14,
            "GW004",
            "attribute 'other' is not declared in the body of class 'Child'"
        )]
    );
}

#[test]
fn attribute_of_a_class_used_as_a_base_is_an_unknown_base() {
    let dir = temp_tree(
        "dotted-base",
        &[(
            "case.py",
            "class A:\n    class Inner:\n        pass\n\n\nclass C(A.Inner):\n    def __init__(self) -> None:\n        self.value = 1\n",
        )],
    );
    let (code, report) = check_json(&dir);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(code, Some(0));
    assert_eq!(presence_rows(&report), []);
}

#[test]
fn call_as_a_base_is_an_unknown_base() {
    let dir = temp_tree(
        "call-base",
        &[(
            "case.py",
            "def make_base() -> type:\n    return object\n\n\nclass C(make_base()):\n    def __init__(self) -> None:\n        self.value = 1\n",
        )],
    );
    let (code, report) = check_json(&dir);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(code, Some(0));
    assert_eq!(presence_rows(&report), []);
}

#[test]
fn annotation_of_an_attribute_in_a_class_body_declares_no_name() {
    let dir = temp_tree(
        "attribute-annotation",
        &[(
            "case.py",
            "class Holder:\n    holder.field: int = 0\n\n\nclass Child(Holder):\n    def __init__(self) -> None:\n        self.field = 1\n",
        )],
    );
    let (code, report) = check_json(&dir);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(code, Some(1));
    assert_eq!(
        presence_rows(&report),
        [(
            "case.py",
            7,
            14,
            "GW004",
            "attribute 'field' is not declared in the body of class 'Child'"
        )]
    );
}

/// Runs the presence phase, `gawain check --no-types --format json`, on `path`. Returns the exit code and
/// the parsed report.
fn check_json(path: &Path) -> (Option<i32>, Value) {
    let output = gawain()
        .args(["check", "--no-types", "--format", "json"])
        .arg(path)
        .output()
        .expect("gawain runs");
    (output.status.code(), json_report(&output.stdout))
}

/// Runs the presence phase like `check_json`, in strict mode, for a test whose diagnostics come from a
/// first binding of an obvious value.
fn check_json_strict(path: &Path) -> (Option<i32>, Value) {
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
fn check_presence_of(contents: &str) -> Output {
    let path = temp_python_file(contents);
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(&path)
        .output()
        .expect("gawain runs");
    fs::remove_file(&path).expect("temp file is removable");
    output
}

#[test]
fn three_thousand_nested_brackets_do_not_abort_the_process() {
    let text = format!("x: int = {}{}\n", "[".repeat(3000), "]".repeat(3000));
    let output = check_presence_of(&text);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[test]
fn five_thousand_unary_minus_signs_do_not_abort_the_process() {
    let text = format!("x: int = {}1\n", "-".repeat(5000));
    let output = check_presence_of(&text);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[test]
fn fifty_thousand_attribute_accesses_do_not_abort_the_process() {
    let text = format!("y: int = x{}\n", ".a".repeat(50000));
    let output = check_presence_of(&text);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_directory_is_gw000_and_the_run_fails() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_tree("unreadable-dir", &[("d/sec/a.py", "x: int = 1\n")]);
    let sec = dir.join("d/sec");
    fs::set_permissions(&sec, fs::Permissions::from_mode(0o000)).expect("permissions are settable");
    if fs::read_dir(&sec).is_ok() {
        // Root reads any directory, so there is no unreadable directory to check.
        fs::set_permissions(&sec, fs::Permissions::from_mode(0o755))
            .expect("permissions are settable");
        fs::remove_dir_all(&dir).expect("temp directory is removable");
        return;
    }
    let output = gawain()
        .args(["check", "--format", "json"])
        .arg(dir.join("d"))
        .output()
        .expect("gawain runs");
    fs::set_permissions(&sec, fs::Permissions::from_mode(0o755)).expect("permissions are settable");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    let diagnostics = report["presence"].as_array().expect("presence is an array");
    assert_eq!(diagnostics.len(), 1, "diagnostics were: {diagnostics:?}");
    assert!(
        diagnostics[0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("/d/sec")),
        "diagnostics were: {diagnostics:?}"
    );
    assert_eq!(
        presence_rows(&report),
        [(
            "sec",
            1,
            1,
            "GW000",
            "cannot read directory: Permission denied (os error 13)"
        )]
    );
}

#[test]
fn two_files_named_alike_outside_a_package_keep_their_own_classes() {
    // Both files are module `settings`. Each child declares its attribute against the base of its own file.
    let dir = temp_tree(
        "same-stem-classes",
        &[
            (
                "x/settings.py",
                "class Base:\n    a: int = 0\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        self.a = 1\n",
            ),
            (
                "y/settings.py",
                "class Base:\n    b: int = 0\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        self.b = 1\n",
            ),
        ],
    );
    let alone = gawain()
        .args(["check", "--no-types"])
        .arg(dir.join("x"))
        .output()
        .expect("gawain runs");
    let forward = gawain()
        .args(["check", "--no-types"])
        .arg(dir.join("x"))
        .arg(dir.join("y"))
        .output()
        .expect("gawain runs");
    let backward = gawain()
        .args(["check", "--no-types"])
        .arg(dir.join("y"))
        .arg(dir.join("x"))
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    for output in [&alone, &forward, &backward] {
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "All checks passed (presence).\n",
            "stderr was: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(0));
    }
}

#[test]
fn import_from_a_module_name_that_two_files_share_releases_the_attribute_check() {
    // `settings` names two files, so `Base` cannot be known and `value` is not reported.
    let dir = temp_tree(
        "ambiguous-import",
        &[
            ("x/settings.py", "class Base:\n    pass\n"),
            ("y/settings.py", "class Other:\n    pass\n"),
            (
                "z/child.py",
                "from settings import Base\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        self.value = 1\n",
            ),
        ],
    );
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(dir.join("x"))
        .arg(dir.join("y"))
        .arg(dir.join("z"))
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "All checks passed (presence).\n"
    );
}

#[test]
fn one_file_named_twice_is_not_an_ambiguous_module_name() {
    // `./settings.py` and `settings.py` are the same file, so `settings` is not ambiguous and `Base` is known.
    let dir = temp_tree(
        "same-file-twice",
        &[
            ("settings.py", "class Base:\n    pass\n"),
            (
                "child.py",
                "from settings import Base\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        self.value = 1\n",
            ),
        ],
    );
    let output = gawain()
        .current_dir(&dir)
        .args([
            "check",
            "--no-types",
            "./settings.py",
            "settings.py",
            "child.py",
        ])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "child.py:6:14  GW004 attribute 'value' is not declared in the body of class 'Child'"
        ),
        "stdout was: {stdout}"
    );
}

#[test]
fn named_directory_that_exclude_lists_is_checked() {
    let dir = temp_tree(
        "named-excluded",
        &[
            ("pyproject.toml", "[tool.gawain]\nexclude = [\"skipped\"]\n"),
            ("skipped/flagged.py", "def f(a): ...\n"),
        ],
    );
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "--no-types", "--format", "json", "skipped"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output.stdout);
    assert_eq!(
        presence_rows(&report),
        [
            (
                "flagged.py",
                1,
                5,
                "GW003",
                "function 'f' has no return type annotation"
            ),
            (
                "flagged.py",
                1,
                7,
                "GW002",
                "parameter 'a' of function 'f' has no type annotation"
            ),
        ]
    );
}

#[test]
fn no_python_files_found_is_said_on_stderr_and_passes() {
    let dir = temp_tree("no-python-files", &[("notes.txt", "def f(a): ...\n")]);
    let output = gawain()
        .args(["check", "--no-types"])
        .arg(&dir)
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "gawain: no Python files found in the given paths\n"
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

#[test]
fn misspelled_key_in_tool_gawain_is_a_usage_error() {
    let dir = temp_tree(
        "config-typo",
        &[
            ("pyproject.toml", "[tool.gawain]\nexlcude = [\"tests\"]\n"),
            ("ok.py", "y: int = 1\n"),
        ],
    );
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "--no-types", "ok.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("exlcude"), "stderr was: {stderr}");
}

// `ulimit -v` is enforced on Linux only.
#[cfg(target_os = "linux")]
#[test]
fn gawain_that_cannot_start_its_thread_exits_2_with_a_message() {
    // A 100 MB address-space limit cannot hold the 256 MiB stack that gawain asks for its thread.
    let path = temp_python_file("y: int = 1\n");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 102400 && exec \"$0\" check --no-types \"$1\"")
        .arg(env!("CARGO_BIN_EXE_gawain"))
        .arg(&path)
        .output()
        .expect("sh runs");
    fs::remove_file(&path).expect("temp file is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("gawain: cannot start: "),
        "stderr was: {stderr}"
    );
}

/// Runs `gawain check --no-types <args> case.py` with `dir` as the working directory, so that the config of
/// `dir` is the one gawain reads.
fn check_case_in(dir: &Path, args: &[&str]) -> Output {
    gawain()
        .current_dir(dir)
        .args(["check", "--no-types"])
        .args(args)
        .arg("case.py")
        .output()
        .expect("gawain runs")
}

#[test]
fn obvious_mode_is_the_default_when_there_is_no_config() {
    let dir = temp_dir_with("default-mode", &[("case.py", b"count = 0\n")]);
    let output = check_case_in(&dir, &[]);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        "All checks passed (presence)."
    );
}

#[test]
fn strict_mode_asks_for_the_annotation_that_obvious_mode_skips() {
    let dir = temp_dir_with("strict-flag", &[("case.py", b"count = 0\n")]);
    let output = check_case_in(&dir, &["--mode", "strict"]);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GW001 variable 'count' has no type annotation"),
        "stdout was: {stdout}"
    );
}

#[test]
fn mode_strict_in_the_config_switches_the_default() {
    let dir = temp_dir_with(
        "config-strict",
        &[
            ("pyproject.toml", b"[tool.gawain]\nmode = \"strict\"\n"),
            ("case.py", b"count = 0\n"),
        ],
    );
    let output = check_case_in(&dir, &[]);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GW001 variable 'count'"),
        "stdout was: {stdout}"
    );
}

#[test]
fn mode_flag_overrides_the_config_in_both_directions() {
    let strict = temp_dir_with(
        "override-strict",
        &[
            ("pyproject.toml", b"[tool.gawain]\nmode = \"strict\"\n"),
            ("case.py", b"count = 0\n"),
        ],
    );
    let obvious = temp_dir_with(
        "override-obvious",
        &[
            ("pyproject.toml", b"[tool.gawain]\nmode = \"obvious\"\n"),
            ("case.py", b"count = 0\n"),
        ],
    );
    let relaxed = check_case_in(&strict, &["--mode", "obvious"]);
    let demanding = check_case_in(&obvious, &["--mode", "strict"]);
    fs::remove_dir_all(&strict).expect("temp directory is removable");
    fs::remove_dir_all(&obvious).expect("temp directory is removable");

    assert_eq!(relaxed.status.code(), Some(0));
    assert_eq!(demanding.status.code(), Some(1));
}

#[test]
fn unknown_mode_is_a_usage_error_in_the_flag_and_in_the_config() {
    let flag_dir = temp_dir_with("bad-flag", &[("case.py", b"count = 0\n")]);
    let flag = check_case_in(&flag_dir, &["--mode", "fancy"]);
    let config_dir = temp_dir_with(
        "bad-config-mode",
        &[
            ("pyproject.toml", b"[tool.gawain]\nmode = \"fancy\"\n"),
            ("case.py", b"count = 0\n"),
        ],
    );
    let config = check_case_in(&config_dir, &[]);
    fs::remove_dir_all(&flag_dir).expect("temp directory is removable");
    fs::remove_dir_all(&config_dir).expect("temp directory is removable");

    assert_eq!(flag.status.code(), Some(2));
    assert_eq!(config.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&config.stderr);
    assert!(stderr.contains("pyproject.toml"), "stderr was: {stderr}");
    assert!(stderr.contains("fancy"), "stderr was: {stderr}");
}

#[test]
fn reassigned_obvious_value_names_the_line_of_the_rebinding() {
    let dir = temp_dir_with(
        "rebound-message",
        &[("case.py", b"count = 0\ncount = count + 1\n")],
    );
    let output = check_case_in(&dir, &[]);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .contains("GW001 variable 'count' has no type annotation (it is reassigned on line 2)"),
        "stdout was: {stdout}"
    );
}

#[test]
fn class_attribute_reassigned_in_the_body_names_the_line_of_the_rebinding() {
    let dir = temp_dir_with(
        "class-rebound-message",
        &[("case.py", b"class Stepper:\n    step = 1\n    step = 2\n")],
    );
    let output = check_case_in(&dir, &[]);
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "GW004 class attribute 'step' of class 'Stepper' has no type annotation (it is reassigned on line 3)"
        ),
        "stdout was: {stdout}"
    );
}
