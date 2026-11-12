use std::fs;

use crate::common::*;

#[test]
fn check_without_paths_is_a_usage_error() {
    gawain().arg("check").assert().code(2);
}

#[test]
fn unknown_checker_on_the_command_line_is_a_usage_error() {
    let output = gawain()
        .args(["check", "--checker", "mypy"])
        .arg(FULLY_ANNOTATED)
        .output()
        .expect("gawain runs");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mypy"), "stderr was: {stderr}");
}

#[test]
fn config_checker_pyrefly_without_pyrefly_is_a_usage_error() {
    let dir = temp_dir_with(
        "config-pyrefly",
        &[
            ("pyproject.toml", b"[tool.gawain]\nchecker = \"pyrefly\"\n"),
            ("case.py", b"y: int = 1\n"),
        ],
    );
    let empty = unique_temp_path("no-pyrefly");
    fs::create_dir_all(&empty).expect("temp directory is creatable");
    let output = gawain()
        .current_dir(&dir)
        .env("PATH", &empty)
        .args(["check", "case.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");
    fs::remove_dir_all(&empty).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(PYREFLY_NOT_FOUND), "stderr was: {stderr}");
}

#[test]
fn config_checker_that_is_unknown_is_a_usage_error() {
    let dir = temp_dir_with(
        "config-mypy",
        &[
            ("pyproject.toml", b"[tool.gawain]\nchecker = \"mypy\"\n"),
            ("case.py", b"y: int = 1\n"),
        ],
    );
    let output = gawain()
        .current_dir(&dir)
        .args(["check", "case.py"])
        .output()
        .expect("gawain runs");
    fs::remove_dir_all(&dir).expect("temp directory is removable");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mypy"), "stderr was: {stderr}");
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
