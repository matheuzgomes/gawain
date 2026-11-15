//! The docs as tests. Each README example must print what the README shows, and every inline assignment in the
//! docs must be registered here, with the result gawain gives it in each mode. A new snippet in the docs fails
//! until it is registered, so no claim about the rules is published unchecked.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use gawain::config::Mode;
use gawain::presence::check_source;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

struct Claim {
    snippet: &'static str,
    /// `None` checks the snippet alone at module level, with `Final` imported.
    source: Option<&'static str>,
    obvious: &'static str,
    strict: &'static str,
}

const fn alone(snippet: &'static str, obvious: &'static str, strict: &'static str) -> Claim {
    Claim {
        snippet,
        source: None,
        obvious,
        strict,
    }
}

const fn within(
    snippet: &'static str,
    source: &'static str,
    obvious: &'static str,
    strict: &'static str,
) -> Claim {
    Claim {
        snippet,
        source: Some(source),
        obvious,
        strict,
    }
}

const RECEIVER: &str =
    "class C:\n    count = 0\n\n    def m(self) -> None:\n        self.count = 1\n";

/// `pass` means no diagnostic. Several codes mean several diagnostics.
const CLAIMS: &[Claim] = &[
    alone("count = 0", "pass", "GW001"),
    alone("name = \"ada\"", "pass", "GW001"),
    alone("ratio = -1.5", "pass", "GW001"),
    alone("flag = True", "pass", "GW001"),
    alone("retries = 3", "pass", "GW001"),
    alone("hits = 0", "pass", "GW001"),
    alone("numbers = [1, 2, 3]", "pass", "GW001"),
    alone("ids = [1, 2, 3]", "pass", "GW001"),
    alone("pair = (1, \"a\")", "pass", "GW001"),
    alone("table = {\"a\": [1]}", "pass", "GW001"),
    alone("user = User(\"ada\")", "pass", "GW001"),
    alone("model = models.User()", "pass", "GW001"),
    alone("words = Counter(lines)", "pass", "GW001"),
    alone("value = None", "GW001", "GW001"),
    alone("value = ...", "GW001", "GW001"),
    alone("value = load()", "GW001", "GW001"),
    alone("mixed = [1, 2.5]", "GW001", "GW001"),
    alone("empty = []", "GW001", "GW001"),
    alone("nested = [[1], [\"a\"]]", "GW001", "GW001"),
    alone("counter = Counter()", "GW001", "GW001"),
    alone("url = URL(\"x\")", "GW001", "GW001"),
    alone("moment = datetime(2020, 1, 1)", "GW001", "GW001"),
    alone("MAX = 3", "GW001", "GW001"),
    alone("MAX: Final = 3", "pass", "GW011"),
    alone("NAME: Final = \"ada\"", "pass", "GW011"),
    alone("PORT: Final = 8080", "pass", "GW011"),
    alone("LIMIT: Final = load()", "GW011", "GW011"),
    alone("x: int = \"a\"", "pass", "pass"),
    within("hits += 1", "hits = 0\nhits += 1\n", "pass", "GW001"),
    within(
        "count = count + 1",
        "count = 0\ncount = count + 1\n",
        "GW001",
        "GW001",
    ),
    within("self.count = 1", RECEIVER, "GW004", "GW004 GW004"),
    within(
        "self.count = ...",
        "class C:\n    count = 0\n\n    def m(self) -> None:\n        self.count = ...\n",
        "GW004",
        "GW004 GW004",
    ),
    within(
        "cls.count = ...",
        "class C:\n    count = 0\n\n    @classmethod\n    def m(cls) -> None:\n        cls.count = ...\n",
        "GW004",
        "GW004 GW004",
    ),
    within(
        "self.count: int = 1",
        "class C:\n    count = 0\n\n    def m(self) -> None:\n        self.count: int = 1\n",
        "GW004 GW004",
        "GW004 GW004",
    ),
    within(
        "T = TypeVar(\"T\")",
        "from typing import TypeVar\n\nT = TypeVar(\"T\")\n",
        "pass",
        "pass",
    ),
    within(
        "self.x = 1",
        "class C:\n    def m(self) -> None:\n        self.x = 1\n",
        "GW004",
        "GW004",
    ),
    within(
        "x: ClassVar[int] = 0",
        "from typing import ClassVar\n\n\nclass C:\n    x: ClassVar[int] = 0\n",
        "pass",
        "pass",
    ),
    within(
        "X: Final[int] = 1",
        "from typing import Final\n\n\nclass C:\n    X: Final[int] = 1\n",
        "pass",
        "pass",
    ),
    within("size = 0", "class C:\n    size = 0\n", "pass", "GW004"),
];

/// Inline assignments that are not Python checked by presence.
const NOT_PRESENCE: &[(&str, &str)] = &[
    ("checker = \"pyrefly\"", "a [tool.gawain] setting"),
    ("checker = \"ty\"", "a [tool.gawain] setting"),
    ("+=", "names the operator"),
    (
        "pyrefly check --output-format json --summary=none --min-severity warn -- FILES",
        "the command gawain runs",
    ),
    ("strict = true", "must-annotate's config"),
    ("exclude = []", "must-annotate's config"),
    ("self.x = ...", "describes must-annotate's TF004"),
];

fn docs() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(Path::new(ROOT).join("docs"))
        .expect("docs/ is readable")
        .map(|entry| entry.expect("a docs/ entry is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .map(|path| format!("docs/{}", path.file_name().unwrap().to_string_lossy()))
        .collect();
    found.sort();
    found.insert(0, "README.md".to_owned());
    found
}

fn read(doc: &str) -> String {
    fs::read_to_string(Path::new(ROOT).join(doc)).expect("the doc is readable")
}

fn inline_code(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            assert!(
                line.matches('`').count() % 2 == 0,
                "a code span is split across lines: {line}"
            );
            spans.extend(line.split('`').skip(1).step_by(2));
        }
    }
    spans
}

fn is_assignment(span: &str) -> bool {
    let rest = span
        .replace("==", "")
        .replace("!=", "")
        .replace("<=", "")
        .replace(">=", "");
    rest.contains('=') && !span.starts_with('-')
}

fn snippets(text: &str) -> Vec<&str> {
    inline_code(text)
        .into_iter()
        .filter(|span| is_assignment(span))
        .collect()
}

fn reported(claim: &Claim, mode: Mode) -> String {
    let source = claim.source.map_or_else(
        || format!("from typing import Final\n\n\n{}\n", claim.snippet),
        str::to_owned,
    );
    let codes: Vec<&str> = check_source(Path::new("case.py"), &source, mode)
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    if codes.is_empty() {
        "pass".to_owned()
    } else {
        codes.join(" ")
    }
}

fn claim(snippet: &str) -> Option<&'static Claim> {
    CLAIMS.iter().find(|claim| claim.snippet == snippet)
}

#[test]
fn every_inline_assignment_in_the_docs_is_registered() {
    let mut unregistered = Vec::new();
    for doc in docs() {
        for snippet in snippets(&read(&doc)) {
            let known =
                claim(snippet).is_some() || NOT_PRESENCE.iter().any(|(other, _)| *other == snippet);
            if !known {
                unregistered.push(format!("{doc}: `{snippet}`"));
            }
        }
    }
    assert!(
        unregistered.is_empty(),
        "register these in CLAIMS with their result in each mode:\n{}",
        unregistered.join("\n")
    );
}

#[test]
fn every_registered_snippet_is_in_the_docs() {
    let all: Vec<String> = docs().iter().map(|doc| read(doc)).collect();
    let in_docs: Vec<&str> = all.iter().flat_map(|text| snippets(text)).collect();
    let registered = CLAIMS
        .iter()
        .map(|claim| claim.snippet)
        .chain(NOT_PRESENCE.iter().map(|(snippet, _)| *snippet));
    let stale: Vec<&str> = registered
        .filter(|snippet| !in_docs.contains(snippet))
        .collect();
    assert!(stale.is_empty(), "no longer in the docs: {stale:?}");
}

#[test]
fn every_claim_gets_its_result_in_both_modes() {
    let mut wrong = Vec::new();
    for claim in CLAIMS {
        for (mode, expected) in [(Mode::Obvious, claim.obvious), (Mode::Strict, claim.strict)] {
            let got = reported(claim, mode);
            if got != expected {
                wrong.push(format!(
                    "`{}` {mode:?}: expected {expected}, got {got}",
                    claim.snippet
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// A cell that reads "`a`, then `b`" or "`a`, with `b`" is one case, judged by its last assignment. Any other cell
/// lists independent cases.
fn cases_in(cell: &str) -> Vec<&str> {
    let found = snippets(cell);
    if cell.contains(", then ") || cell.contains(", with ") {
        found.last().into_iter().copied().collect()
    } else {
        found
    }
}

#[test]
fn the_rules_table_agrees_with_the_claims() {
    let rules = read("docs/rules.md");
    let header = "| Obvious: no annotation needed | Annotation required |";
    let start = rules
        .find(header)
        .expect("docs/rules.md has the modes table");
    let rows = rules[start..]
        .lines()
        .skip(2)
        .take_while(|line| line.starts_with('|'));
    let mut wrong = Vec::new();
    let mut checked = 0;
    for row in rows {
        let cells: Vec<&str> = row.split(" | ").collect();
        let [obvious, required] = cells[..] else {
            panic!("a row of the modes table has two cells: {row}");
        };
        for (cell, passes) in [(obvious, true), (required, false)] {
            for snippet in cases_in(cell) {
                checked += 1;
                let claim = claim(snippet).expect("every snippet is registered");
                if (claim.obvious == "pass") != passes {
                    wrong.push(format!("`{snippet}` is in the wrong column"));
                }
            }
        }
    }
    assert!(checked > 0, "no case was read from the modes table");
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

struct Example {
    file: String,
    source: String,
    command: String,
    output: String,
}

/// Reads each "Given `file`:", its python block, "`gawain check …` prints:" and its text block.
fn readme_examples(readme: &str) -> Vec<Example> {
    let mut lines = readme.lines();
    let mut examples = Vec::new();
    while let Some(line) = lines.next() {
        let Some(file) = line
            .split_once("Given `")
            .and_then(|(_, rest)| rest.strip_suffix("`:"))
        else {
            continue;
        };
        let source = fenced_block(&mut lines, "```python");
        let command = lines
            .find_map(|line| line.strip_prefix('`')?.strip_suffix("` prints:"))
            .expect("an example says what command prints its output");
        let output = fenced_block(&mut lines, "```text");
        examples.push(Example {
            file: file.to_owned(),
            source,
            command: command.to_owned(),
            output,
        });
    }
    examples
}

fn fenced_block<'a>(lines: &mut impl Iterator<Item = &'a str>, fence: &str) -> String {
    lines
        .find(|line| *line == fence)
        .unwrap_or_else(|| panic!("an example has a {fence} block"));
    let mut block = String::new();
    for line in lines.by_ref().take_while(|line| *line != "```") {
        block.push_str(line);
        block.push('\n');
    }
    block
}

fn unique_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_nanos();
    std::env::temp_dir().join(format!("gawain-docs-{}-{nanos}-{name}", std::process::id()))
}

/// The types example runs ty 0.0.100 from the project venv, as the real-ty CLI tests do.
fn path_with_venv() -> std::ffi::OsString {
    let bin = Path::new(ROOT).join(".venv/bin");
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

#[test]
fn every_readme_example_prints_what_the_readme_shows() {
    let readme = read("README.md");
    let examples = readme_examples(&readme);
    assert_eq!(
        examples.len(),
        readme.matches("Given `").count(),
        "every example was read"
    );
    assert!(!examples.is_empty(), "the README has examples");

    for example in examples {
        assert_eq!(
            example.command.split_whitespace().next(),
            Some("gawain"),
            "README example command starts with gawain: `{}`",
            example.command
        );
        assert!(
            !example.command.contains(['"', '\'']),
            "README example commands run without a shell: `{}`",
            example.command
        );
        let dir = unique_dir(&example.file);
        fs::create_dir_all(&dir).expect("temp directory is creatable");
        fs::write(dir.join(&example.file), &example.source).expect("temp file is writable");
        let args: Vec<&str> = example.command.split_whitespace().skip(1).collect();
        let output = Command::cargo_bin("gawain")
            .expect("the gawain binary is built")
            .current_dir(&dir)
            .env("PATH", path_with_venv())
            .args(&args)
            .output()
            .expect("gawain runs");
        fs::remove_dir_all(&dir).expect("temp directory is removable");

        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            example.output,
            "`{}` in the README",
            example.command
        );
    }
}
