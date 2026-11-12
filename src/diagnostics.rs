use std::path::{Path, PathBuf};

use serde::Serialize;

/// Stable identifier of one kind of diagnostic, such as `GW001`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Code {
    /// A file or directory that cannot be checked: a file is not valid UTF-8, cannot be read, or has a
    /// syntax error, and a directory cannot be listed.
    GW000,
    GW001,
    GW002,
    GW003,
    GW004,
    GW010,
    GW011,
    GW020,
    GW900,
    GW901,
}

impl Code {
    const ALL: [Code; 10] = [
        Code::GW000,
        Code::GW001,
        Code::GW002,
        Code::GW003,
        Code::GW004,
        Code::GW010,
        Code::GW011,
        Code::GW020,
        Code::GW900,
        Code::GW901,
    ];

    /// The text of each code, in the order the variants are declared, which `as_str` indexes by
    /// discriminant. The round-trip test checks that the two orders agree.
    const TEXT: [&'static str; 10] = [
        "GW000", "GW001", "GW002", "GW003", "GW004", "GW010", "GW011", "GW020", "GW900", "GW901",
    ];

    pub fn as_str(self) -> &'static str {
        Code::TEXT[self as usize]
    }

    pub fn parse(text: &str) -> Option<Code> {
        Code::ALL.into_iter().find(|code| code.as_str() == text)
    }
}

/// One reported problem. Field order matters: the derived `Ord` sorts by path, then line, column and code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, counted in characters from the start of the line.
    pub col: u32,
    pub code: Code,
    pub message: String,
}

impl Diagnostic {
    /// A GW000 for a path that cannot be checked as a whole, such as a file that cannot be read or decoded,
    /// or a directory that cannot be listed. It has no position in the text, so it sits at line 1, column 1.
    pub fn for_path(path: &Path, message: String) -> Self {
        Self {
            path: path.to_path_buf(),
            line: 1,
            col: 1,
            code: Code::GW000,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Code;

    #[test]
    fn every_code_parses_back_from_its_text() {
        for code in Code::ALL {
            assert_eq!(Code::parse(code.as_str()), Some(code));
        }
    }

    #[test]
    fn gw000_is_the_code_of_a_file_that_cannot_be_checked() {
        assert_eq!(Code::GW000.as_str(), "GW000");
        assert_eq!(Code::parse("GW000"), Some(Code::GW000));
    }
}
