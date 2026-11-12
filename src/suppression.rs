//! Line suppressions. A `gawain: ignore[CODE, ...]` comment silences the listed codes on the line that
//! it sits on, and nothing else. A malformed directive is reported as GW900, and a listed code that
//! silences nothing as GW901. Neither can be silenced.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ruff_source_file::SourceCode;
use ruff_text_size::TextRange;

use crate::diagnostics::{Code, Diagnostic};
use crate::presence::line_col;

/// The word that begins a directive. It is followed by a colon, optional spaces, and `ignore`.
const NAME: &str = "gawain";
const KEYWORD: &str = "ignore";
const NEEDS_CODE: &str = "'gawain: ignore' needs at least one code, e.g. '# gawain: ignore[GW001]'";

/// The suppressions of one file: the codes that each line silences, the codes that each directive
/// lists, and the GW900 errors for malformed directives.
#[derive(Debug, Default)]
pub struct Suppressions {
    by_line: HashMap<u32, Vec<Code>>,
    listed: Vec<Listed>,
    errors: Vec<Diagnostic>,
}

/// A code that a valid directive lists, at `line`. `unused` is the GW901 that is reported when no
/// diagnostic on that line has the code.
#[derive(Debug)]
struct Listed {
    line: u32,
    code: Code,
    unused: Diagnostic,
}

/// Reads the suppression directives of one file. `comments` holds the comment tokens with their ranges
/// and text, so the text of a string literal is never read as a comment.
pub fn parse_suppressions<'a>(
    path: &Path,
    comments: impl Iterator<Item = (TextRange, &'a str)>,
    code: &SourceCode,
) -> Suppressions {
    let mut suppressions = Suppressions::default();
    for (range, text) in comments {
        let Some(listed) = listed_codes(text) else {
            continue;
        };
        let (line, col) = line_col(code, range.start());
        suppressions.record(path, line, col, &listed);
    }
    suppressions
}

/// Drops the diagnostics that a suppression silences, then appends the GW900 errors and the GW901 reports
/// for listed codes that silenced nothing. These are added after the filter, so no suppression can
/// silence them.
pub fn apply(diagnostics: Vec<Diagnostic>, suppressions: &Suppressions) -> Vec<Diagnostic> {
    let used: HashSet<(u32, Code)> = diagnostics
        .iter()
        .filter(|diagnostic| suppressions.silences(diagnostic))
        .map(|diagnostic| (diagnostic.line, diagnostic.code))
        .collect();
    let mut kept: Vec<Diagnostic> = diagnostics
        .into_iter()
        .filter(|diagnostic| !suppressions.silences(diagnostic))
        .collect();
    kept.extend(suppressions.errors.iter().cloned());
    kept.extend(
        suppressions
            .listed
            .iter()
            .filter(|listed| !used.contains(&(listed.line, listed.code)))
            .map(|listed| listed.unused.clone()),
    );
    kept
}

impl Suppressions {
    /// Adds the codes of one directive, which sits at `line` and `col`. A directive that lists no codes
    /// and each entry that is not a suppressible code are GW900 errors at the start of the comment. Each
    /// suppressible code is listed once, so a code repeated in one directive is reported once if unused.
    fn record(&mut self, path: &Path, line: u32, col: u32, listed: &[&str]) {
        if listed.is_empty() {
            self.errors
                .push(error(path, line, col, NEEDS_CODE.to_owned()));
        }
        for entry in listed {
            self.record_entry(path, line, col, entry);
        }
    }

    /// Records one entry of a directive. A suppressible code is listed, and any other entry is a GW900.
    fn record_entry(&mut self, path: &Path, line: u32, col: u32, entry: &str) {
        match suppressible(entry) {
            Some(code) => self.list(path, line, col, code),
            None => self.errors.push(error(
                path,
                line,
                col,
                format!("unknown code '{entry}' in 'gawain: ignore'"),
            )),
        }
    }

    /// Records that the directive at `line` and `col` lists `code`, unless it already does.
    fn list(&mut self, path: &Path, line: u32, col: u32, code: Code) {
        let codes = self.by_line.entry(line).or_default();
        if codes.contains(&code) {
            return;
        }
        codes.push(code);
        self.listed.push(Listed {
            line,
            code,
            unused: diagnostic(
                path,
                line,
                col,
                Code::GW901,
                format!("unused suppression '{}'", code.as_str()),
            ),
        });
    }

    /// Whether a suppression on the line of `diagnostic` lists its code.
    fn silences(&self, diagnostic: &Diagnostic) -> bool {
        self.by_line
            .get(&diagnostic.line)
            .is_some_and(|codes| codes.contains(&diagnostic.code))
    }
}

/// The code that a listed entry names, when a suppression may silence it. GW000, GW900 and GW901 are not.
fn suppressible(entry: &str) -> Option<Code> {
    Code::parse(entry).filter(|code| !matches!(code, Code::GW000 | Code::GW900 | Code::GW901))
}

fn error(path: &Path, line: u32, col: u32, message: String) -> Diagnostic {
    diagnostic(path, line, col, Code::GW900, message)
}

fn diagnostic(path: &Path, line: u32, col: u32, code: Code, message: String) -> Diagnostic {
    Diagnostic {
        path: path.to_path_buf(),
        line,
        col,
        code,
        message,
    }
}

/// The codes that the first `gawain: ignore` directive of a comment lists, trimmed and without empty
/// entries. `None` when the comment holds no directive. A directive without a closed pair of brackets
/// lists no codes.
fn listed_codes(comment: &str) -> Option<Vec<&str>> {
    let rest = comment
        .match_indices(NAME)
        .filter(|(at, _)| starts_directive(comment, *at))
        .find_map(|(at, _)| after_keyword(&comment[at + NAME.len()..]))?;
    let inside = rest
        .trim_start()
        .strip_prefix('[')
        .and_then(|inside| inside.split_once(']'))
        .map_or("", |(list, _)| list);
    Some(
        inside
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect(),
    )
}

/// Whether the word `gawain` at byte `at` of `comment` may begin a directive. It must follow whitespace or
/// a `#`, so `mygawain: ignore` is not one. The comment starts with its own `#`, so `#gawain: ignore` passes.
fn starts_directive(comment: &str, at: usize) -> bool {
    comment[..at].ends_with(|c: char| c == '#' || c.is_whitespace())
}

/// The text after `ignore`, when `text`, which follows `gawain`, reads `: ignore` with optional spaces
/// around the colon. The word must end there, so `gawain: ignored` is not a directive.
fn after_keyword(text: &str) -> Option<&str> {
    let rest = text
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix(KEYWORD)?;
    let ends_word = !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_');
    ends_word.then_some(rest)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::NEEDS_CODE;
    use crate::config::Mode;
    use crate::diagnostics::Code;
    use crate::presence::check_source;

    /// The `(line, col, message)` of every diagnostic with `code` that `source` produces.
    fn reports(source: &str, code: Code) -> Vec<(u32, u32, String)> {
        check_source(Path::new("case.py"), source, Mode::Strict)
            .into_iter()
            .filter(|diagnostic| diagnostic.code == code)
            .map(|diagnostic| (diagnostic.line, diagnostic.col, diagnostic.message))
            .collect()
    }

    #[test]
    fn a_listed_code_that_silences_nothing_is_gw901_at_the_comment_start() {
        let source = "x: int = 1  # gawain: ignore[GW001]\n";
        assert_eq!(
            reports(source, Code::GW901),
            [(1, 13, "unused suppression 'GW001'".to_owned())]
        );
    }

    #[test]
    fn a_used_code_is_not_reported_as_unused() {
        assert_eq!(reports("a = 1  # gawain: ignore[GW001]\n", Code::GW901), []);
    }

    #[test]
    fn only_the_unused_codes_of_a_directive_are_reported() {
        let source = "def g(x): ...  # gawain: ignore[GW002,GW003,GW001]\n";
        assert_eq!(
            reports(source, Code::GW901),
            [(1, 16, "unused suppression 'GW001'".to_owned())]
        );
    }

    #[test]
    fn an_unused_code_listed_twice_is_reported_once() {
        let source = "x: int = 1  # gawain: ignore[GW001, GW001]\n";
        assert_eq!(reports(source, Code::GW901).len(), 1);
    }

    #[test]
    fn gw901_cannot_be_listed_so_it_is_an_unknown_code() {
        let source = "t = 1  # gawain: ignore[GW901]\n";
        assert_eq!(
            reports(source, Code::GW900),
            [(1, 8, "unknown code 'GW901' in 'gawain: ignore'".to_owned())]
        );
        assert_eq!(reports(source, Code::GW901), []);
    }

    #[test]
    fn unknown_entries_and_a_missing_code_each_report_their_own_gw900() {
        let source = "x: int = 1  # gawain: ignore[GW999, gw001]\ny: int = 1  # gawain: ignore\n";
        let diagnostics = check_source(Path::new("case.py"), source, Mode::Strict);
        let messages: Vec<&str> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == Code::GW900)
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        assert_eq!(
            messages,
            [
                "unknown code 'GW999' in 'gawain: ignore'",
                "unknown code 'gw001' in 'gawain: ignore'",
                NEEDS_CODE,
            ]
        );
    }
}
