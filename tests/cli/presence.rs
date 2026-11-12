use std::fs;

use crate::common::*;

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
