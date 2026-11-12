use std::fs;

use serde_json::{Value, json};

use crate::common::*;

#[test]
fn json_types_is_null_with_no_types() {
    let output = gawain()
        .args(["check", "--no-types", "--format", "json", FULLY_ANNOTATED])
        .output()
        .expect("gawain runs");

    assert_eq!(output.status.code(), Some(0));
    let report = json_report(&output.stdout);
    assert!(report.get("checker").is_some_and(Value::is_null));
    assert_eq!(report["presence"], json!([]));
    assert_eq!(report["types"], Value::Null);
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
