use std::fs;

use crate::common::*;

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
