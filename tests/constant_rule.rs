//! Every way to write a constant-like assignment, in every scope and mode, against the code that
//! `docs/rules.md` says it gets. The grid must hold each combination of the dimensions exactly once, so a
//! case cannot be left out by forgetting it.

use std::collections::BTreeSet;
use std::path::Path;

use gawain::config::Mode;
use gawain::presence::check_source;

const NAMES: [&str; 2] = ["PORT", "port"];
const ANNOTATIONS: [&str; 4] = ["", ": int", ": Final", ": Final[int]"];
const VALUES: [&str; 2] = ["8080", "load()"];

const SCOPES: [Scope; 3] = [Scope::Module, Scope::Class, Scope::Function];
const MODES: [Mode; 2] = [Mode::Obvious, Mode::Strict];

/// One row per statement. The columns follow `SCOPES` then `MODES`: module obvious, module strict, class
/// obvious, class strict, function obvious, function strict. `pass` means no diagnostic.
const EXPECTED: [(&str, [&str; 6]); 16] = [
    (
        "PORT = 8080",
        ["GW001", "GW001", "GW004", "GW004", "pass", "GW001"],
    ),
    (
        "PORT = load()",
        ["GW001", "GW001", "GW004", "GW004", "GW001", "GW001"],
    ),
    (
        "PORT: int = 8080",
        ["GW010", "GW010", "GW010", "GW010", "pass", "pass"],
    ),
    (
        "PORT: int = load()",
        ["GW010", "GW010", "GW010", "GW010", "pass", "pass"],
    ),
    (
        "PORT: Final = 8080",
        ["pass", "GW011", "pass", "GW011", "pass", "GW011"],
    ),
    (
        "PORT: Final = load()",
        ["GW011", "GW011", "GW011", "GW011", "GW011", "GW011"],
    ),
    (
        "PORT: Final[int] = 8080",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
    (
        "PORT: Final[int] = load()",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
    (
        "port = 8080",
        ["pass", "GW001", "pass", "GW004", "pass", "GW001"],
    ),
    (
        "port = load()",
        ["GW001", "GW001", "GW004", "GW004", "GW001", "GW001"],
    ),
    (
        "port: int = 8080",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
    (
        "port: int = load()",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
    (
        "port: Final = 8080",
        ["pass", "GW011", "pass", "GW011", "pass", "GW011"],
    ),
    (
        "port: Final = load()",
        ["GW011", "GW011", "GW011", "GW011", "GW011", "GW011"],
    ),
    (
        "port: Final[int] = 8080",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
    (
        "port: Final[int] = load()",
        ["pass", "pass", "pass", "pass", "pass", "pass"],
    ),
];

#[derive(Debug, Clone, Copy)]
enum Scope {
    Module,
    Class,
    Function,
}

fn source(statement: &str, scope: Scope) -> String {
    let body = match scope {
        Scope::Module => statement.to_owned(),
        Scope::Class => format!("class C:\n    {statement}"),
        Scope::Function => format!("def f() -> None:\n    {statement}"),
    };
    format!("from typing import Final\n\n\n{body}\n")
}

fn reported(statement: &str, scope: Scope, mode: Mode) -> String {
    let codes: Vec<&str> = check_source(Path::new("case.py"), &source(statement, scope), mode)
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    if codes.is_empty() {
        "pass".to_owned()
    } else {
        codes.join(" ")
    }
}

#[test]
fn the_grid_holds_every_combination_exactly_once() {
    let mut every = BTreeSet::new();
    for name in NAMES {
        for annotation in ANNOTATIONS {
            for value in VALUES {
                every.insert(format!("{name}{annotation} = {value}"));
            }
        }
    }
    let listed: Vec<String> = EXPECTED
        .iter()
        .map(|(statement, _)| (*statement).to_owned())
        .collect();
    let unique: BTreeSet<String> = listed.iter().cloned().collect();

    assert_eq!(unique.len(), listed.len(), "a statement is listed twice");
    let missing: Vec<&String> = every.difference(&unique).collect();
    let unknown: Vec<&String> = unique.difference(&every).collect();
    assert!(missing.is_empty(), "not in the grid: {missing:?}");
    assert!(
        unknown.is_empty(),
        "not a combination of the dimensions: {unknown:?}"
    );
}

#[test]
fn every_statement_gets_the_documented_code_in_every_scope_and_mode() {
    let mut wrong = Vec::new();
    for (statement, codes) in EXPECTED {
        let columns = SCOPES
            .iter()
            .flat_map(|scope| MODES.iter().map(move |mode| (*scope, *mode)));
        for ((scope, mode), expected) in columns.zip(codes) {
            let got = reported(statement, scope, mode);
            if got != expected {
                wrong.push(format!(
                    "{statement:<28} {scope:?} {mode:?}: expected {expected}, got {got}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}
