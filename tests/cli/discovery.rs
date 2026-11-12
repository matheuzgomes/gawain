use std::fs;
use std::path::Path;

use crate::common::*;

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
