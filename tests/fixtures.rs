//! Runs gawain over the fixtures and compares what it reports with the `# expect: CODE ...` markers. A line
//! without a marker expects nothing. Pairs are counted, so `# expect: GW001 GW001` means two GW001
//! diagnostics on that line.
//!
//! There are two fixture sets, each run in its own mode. `tests/fixtures` runs with `--mode strict`, and
//! `tests/fixtures_obvious` runs with `--mode obvious`.
//!
//! In each set, every `*.py` file is one run on that file. Each subdirectory is one run on the whole
//! directory, and its markers come from every `.py` file inside it, recursively. Names in the mismatch
//! reports are relative to the set's directory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use gawain::diagnostics::Code;
use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const OBVIOUS_FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures_obvious");
const MARKER: &str = "# expect:";

/// A fixture set: the directory that holds it, and the `--mode` that gawain runs it in.
struct Suite {
    dir: &'static str,
    mode: &'static str,
}

const STRICT: Suite = Suite {
    dir: FIXTURES,
    mode: "strict",
};
const OBVIOUS: Suite = Suite {
    dir: OBVIOUS_FIXTURES,
    mode: "obvious",
};

/// A problem as `(file, line, code)`, where the file is relative to the fixture set's directory.
type Key = (String, u32, Code);

/// How many times each `Key` occurs.
type Counts = BTreeMap<Key, usize>;

#[test]
fn file_fixtures_match_their_expect_markers() {
    file_fixtures(&STRICT);
}

#[test]
fn directory_fixtures_match_their_expect_markers() {
    directory_fixtures(&STRICT);
}

#[test]
fn obvious_file_fixtures_match_their_expect_markers() {
    file_fixtures(&OBVIOUS);
}

#[test]
fn obvious_directory_fixtures_match_their_expect_markers() {
    directory_fixtures(&OBVIOUS);
}

fn file_fixtures(suite: &Suite) {
    let base = PathBuf::from(suite.dir);
    let fixtures = fixture_entries(suite, is_python);
    assert!(!fixtures.is_empty(), "no fixtures found in {}", suite.dir);

    let failures: Vec<String> = fixtures
        .iter()
        .filter_map(|path| check_run(suite, path, &base, std::slice::from_ref(path)).err())
        .collect();
    assert_no_failures(suite, "fixture file(s)", &failures);
}

fn directory_fixtures(suite: &Suite) {
    let base = PathBuf::from(suite.dir);
    let directories = fixture_entries(suite, |path| path.is_dir());
    assert!(
        !directories.is_empty(),
        "no fixture directories found in {}",
        suite.dir
    );

    let failures: Vec<String> = directories
        .iter()
        .filter_map(|dir| check_run(suite, dir, &base, &python_files_under(dir)).err())
        .collect();
    assert_no_failures(suite, "fixture directory(ies)", &failures);
}

fn assert_no_failures(suite: &Suite, kind: &str, failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} {kind} in {} (--mode {}) do not match their expect markers:\n{}",
        failures.len(),
        suite.dir,
        suite.mode,
        failures.join("\n")
    );
}

fn is_python(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "py")
}

/// The entries of the fixture set's directory that pass `keep`, sorted.
fn fixture_entries(suite: &Suite, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(suite.dir)
        .expect("fixtures directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| keep(path))
        .collect();
    entries.sort();
    entries
}

/// Every `.py` file under `dir`, recursively, sorted.
fn python_files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("fixture directory is readable") {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if is_python(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// The path of `path` relative to `base`, or the whole path when it is not under `base`.
fn relative_name(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Runs gawain in the suite's mode on `target` and compares its output with the markers in `sources`. Returns
/// a report listing every mismatch, or `Ok` when the run matches.
fn check_run(suite: &Suite, target: &Path, base: &Path, sources: &[PathBuf]) -> Result<(), String> {
    let name = relative_name(target, base);
    let mut expected = Counts::new();
    for source in sources {
        let source_name = relative_name(source, base);
        let text = fs::read_to_string(source)
            .map_err(|error| format!("{name}: cannot read {source_name}: {error}"))?;
        let counts =
            expected_counts(&source_name, &text).map_err(|error| format!("{name}: {error}"))?;
        expected.extend(counts);
    }

    let output = Command::cargo_bin("gawain")
        .expect("the gawain binary is built for integration tests")
        .args([
            "check",
            "--no-types",
            "--format",
            "json",
            "--mode",
            suite.mode,
        ])
        .arg(target)
        .output()
        .map_err(|error| format!("{name}: cannot run gawain: {error}"))?;

    let actual_exit = output.status.code();
    if !matches!(actual_exit, Some(0 | 1)) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{name}: gawain exited with {actual_exit:?}: {}",
            stderr.trim()
        ));
    }
    let actual =
        reported_counts(&output.stdout, base).map_err(|error| format!("{name}: {error}"))?;

    let expected_exit = if expected.is_empty() { 0 } else { 1 };
    let mut problems = Vec::new();
    if actual_exit != Some(expected_exit) {
        problems.push(format!(
            "  exit code: expected {expected_exit}, got {actual_exit:?}"
        ));
    }
    problems.extend(count_mismatches(&expected, &actual));

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{name}:\n{}", problems.join("\n")))
    }
}

/// One problem line for every key whose expected count differs from the reported count.
fn count_mismatches(expected: &Counts, actual: &Counts) -> Vec<String> {
    let keys: BTreeSet<&Key> = expected.keys().chain(actual.keys()).collect();
    let mut problems = Vec::new();
    for key @ (file, line, code) in keys {
        let wanted = expected.get(key).copied().unwrap_or(0);
        let found = actual.get(key).copied().unwrap_or(0);
        if wanted > found {
            problems.push(format!(
                "  missing:    ({file}, {line}, {}) x{}",
                code.as_str(),
                wanted - found
            ));
        } else if found > wanted {
            problems.push(format!(
                "  unexpected: ({file}, {line}, {}) x{}",
                code.as_str(),
                found - wanted
            ));
        }
    }
    problems
}

/// Reads every `# expect: CODE ...` marker in `source`, for the file called `name`.
fn expected_counts(name: &str, source: &str) -> Result<Counts, String> {
    let mut counts = Counts::new();
    for (index, text) in source.lines().enumerate() {
        let Some((_, rest)) = text.split_once(MARKER) else {
            continue;
        };
        let line = u32::try_from(index + 1).expect("fixture line number fits in u32");
        for word in rest.split_whitespace() {
            let code = Code::parse(word)
                .ok_or_else(|| format!("line {line}: unknown code '{word}' in expect marker"))?;
            *counts.entry((name.to_owned(), line, code)).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

/// Reads the presence diagnostics in gawain's JSON output as counted keys. Paths are made relative to `base`.
fn reported_counts(stdout: &[u8], base: &Path) -> Result<Counts, String> {
    let report: Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("stdout is not a JSON object: {error}"))?;
    let diagnostics = report["presence"]
        .as_array()
        .ok_or_else(|| "stdout has no `presence` array".to_owned())?;
    let mut counts = Counts::new();
    for diagnostic in diagnostics {
        let file = diagnostic["path"]
            .as_str()
            .map(|path| relative_name(Path::new(path), base));
        let line = diagnostic["line"]
            .as_u64()
            .and_then(|line| u32::try_from(line).ok());
        let code = diagnostic["code"].as_str().and_then(Code::parse);
        match (file, line, code) {
            (Some(file), Some(line), Some(code)) => {
                *counts.entry((file, line, code)).or_insert(0) += 1;
            }
            _ => return Err(format!("unrecognised diagnostic: {diagnostic}")),
        }
    }
    Ok(counts)
}
