use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

use crate::config::TypeChecker;
use crate::diagnostics::Diagnostic;
use crate::types_phase::{Severity, TypeDiagnostic, TypesRun};

/// Writes the report for a terminal, without a trailing newline. Each row is written as soon as it is
/// formatted, so the report is never built in memory as one string.
pub fn write_text(
    out: &mut impl Write,
    presence: &[Diagnostic],
    types: Option<&TypesRun>,
) -> io::Result<()> {
    if !presence.is_empty() {
        return write_presence(out, presence);
    }
    write_types_outcome(out, types)
}

fn write_types_outcome(out: &mut impl Write, types: Option<&TypesRun>) -> io::Result<()> {
    match types {
        None => out.write_all(b"All checks passed (presence)."),
        Some(run) if types_passed(run) => out.write_all(b"All checks passed (presence, types)."),
        Some(run) => write_types(out, run),
    }
}

fn types_passed(run: &TypesRun) -> bool {
    run.diagnostics.is_empty() && !run.failed
}

/// Renders diagnostics as one JSON object with a `checker` name, a `presence` array and a `types` array.
/// `checker` and `types` are `null` when the types phase did not run.
pub fn render_json(
    presence: &[Diagnostic],
    types: Option<&TypesRun>,
) -> Result<String, serde_json::Error> {
    let report = JsonReport {
        checker: types.map(|run| run.checker.name()),
        presence,
        types: types.map(|run| run.diagnostics.as_slice()),
    };
    serde_json::to_string_pretty(&report)
}

#[derive(Serialize)]
struct JsonReport<'a> {
    checker: Option<&'static str>,
    presence: &'a [Diagnostic],
    types: Option<&'a [TypeDiagnostic]>,
}

fn write_presence(out: &mut impl Write, diags: &[Diagnostic]) -> io::Result<()> {
    let noun = if diags.len() == 1 { "error" } else { "errors" };
    let summary = format!(
        "{} {noun} (presence). Type check skipped until presence errors are fixed.",
        diags.len()
    );
    write_section(out, "[presence]", diags, &summary)
}

fn write_types(out: &mut impl Write, run: &TypesRun) -> io::Result<()> {
    let rows: Vec<TypeRow<'_>> = run
        .diagnostics
        .iter()
        .map(|diagnostic| TypeRow {
            checker: run.checker,
            diagnostic,
        })
        .collect();
    write_section(out, "[types]", &rows, &types_summary(run))
}

fn types_summary(run: &TypesRun) -> String {
    let warnings = run
        .diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Warning)
        .count();
    let errors = run.diagnostics.len() - warnings;
    let parts: Vec<String> = [
        counted(errors, "error", "errors"),
        counted(warnings, "warning", "warnings"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        format!("{} failed without diagnostics (types).", run.checker.name())
    } else {
        format!("{} (types).", parts.join(", "))
    }
}

fn counted(count: usize, one: &str, many: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    }
}

trait Row {
    fn path(&self) -> &Path;
    fn line(&self) -> u32;
    fn col(&self) -> u32;
    /// Writes the text after the location, such as `GW001 variable 'x' has no type annotation`.
    fn write_text(&self, out: &mut dyn Write) -> io::Result<()>;
}

impl Row for Diagnostic {
    fn path(&self) -> &Path {
        &self.path
    }

    fn line(&self) -> u32 {
        self.line
    }

    fn col(&self) -> u32 {
        self.col
    }

    fn write_text(&self, out: &mut dyn Write) -> io::Result<()> {
        write!(out, "{} {}", self.code.as_str(), self.message)
    }
}

/// A diagnostic of the types phase as a row. Its text starts with the checker's name, such as `ty[rule]`.
struct TypeRow<'a> {
    checker: TypeChecker,
    diagnostic: &'a TypeDiagnostic,
}

impl Row for TypeRow<'_> {
    fn path(&self) -> &Path {
        &self.diagnostic.path
    }

    fn line(&self) -> u32 {
        self.diagnostic.line
    }

    fn col(&self) -> u32 {
        self.diagnostic.col
    }

    fn write_text(&self, out: &mut dyn Write) -> io::Result<()> {
        write!(
            out,
            "{}[{}] {}",
            self.checker.name(),
            self.diagnostic.rule,
            self.diagnostic.message
        )
    }
}

fn write_section<R: Row>(
    out: &mut impl Write,
    title: &str,
    rows: &[R],
    summary: &str,
) -> io::Result<()> {
    writeln!(out, "{title}")?;
    let widths: Vec<usize> = rows.iter().map(location_width).collect();
    let width = widths.iter().copied().max().unwrap_or_default();
    for (row, location) in rows.iter().zip(widths) {
        write_row(out, row, width - location)?;
    }
    write!(out, "\n{summary}")
}

fn write_row(out: &mut impl Write, row: &impl Row, padding: usize) -> io::Result<()> {
    write_location(out, row)?;
    write_spaces(out, padding + 2)?;
    row.write_text(out)?;
    out.write_all(b"\n")
}

fn write_location(out: &mut impl Write, row: &impl Row) -> io::Result<()> {
    out.write_all(row.path().to_string_lossy().as_bytes())?;
    write!(out, ":{}:{}", row.line(), row.col())
}

/// Writes `count` spaces, from a buffer of spaces rather than one character at a time.
fn write_spaces(out: &mut impl Write, mut count: usize) -> io::Result<()> {
    const SPACES: [u8; 64] = [b' '; 64];
    while count > 0 {
        let chunk = count.min(SPACES.len());
        out.write_all(&SPACES[..chunk])?;
        count -= chunk;
    }
    Ok(())
}

fn location_width(row: &impl Row) -> usize {
    row.path().to_string_lossy().chars().count()
        + 2
        + decimal_digits(row.line())
        + decimal_digits(row.col())
}

fn decimal_digits(value: u32) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}
