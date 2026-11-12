use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::{DirEntry, WalkBuilder};

use crate::diagnostics::Diagnostic;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// The Python files, sorted. A file reached in two ways, such as `./a.py` and `a.py`, or through a symbolic
    /// link to it, is listed once.
    pub files: Vec<PathBuf>,
    pub errors: Vec<Diagnostic>,
}

/// Expands `paths` into the Python files to check, and collects the walk errors.
///
/// A path named on the command line is always checked. `exclude` applies only to paths found while walking:
/// with `exclude = ["tests/fixtures"]`, `gawain check .` skips `tests/fixtures`, while `gawain check
/// tests/fixtures` checks it. A directory named inside an excluded path is walked in full, for the same reason.
///
/// A path that the walk cannot read gives a GW000 at its start, so the run fails rather than passing
/// over what it could not see.
pub fn python_files(paths: &[PathBuf], exclude: &[String]) -> Discovery {
    let cwd = std::env::current_dir().ok();
    let excluded: Vec<PathBuf> = exclude
        .iter()
        .map(|entry| normalize(Path::new(entry), cwd.as_deref()))
        .collect();

    let mut found = Discovery::default();
    for path in paths {
        found.absorb(expand(path, cwd.as_deref(), &excluded));
    }
    // A file reached by several paths keeps a path that is not a link when there is one, so that the real
    // file keeps its own module name for imports. Otherwise the first path in sorted order is kept. The
    // files are sorted again so that the output keeps the global order.
    found
        .files
        .sort_by_cached_key(|file| (is_symlink(file), file.clone()));
    let mut seen = HashSet::new();
    found.files.retain(|file| seen.insert(identity(file)));
    found.files.sort();
    found
}

/// The path of a file with symbolic links and `.` or `..` components resolved, so that two spellings of
/// one file compare equal. A path that cannot be resolved is compared as it is.
fn identity(file: &Path) -> PathBuf {
    file.canonicalize().unwrap_or_else(|_| file.to_path_buf())
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

impl Discovery {
    fn absorb(&mut self, other: Discovery) {
        self.files.extend(other.files);
        self.errors.extend(other.errors);
    }
}

fn expand(path: &Path, cwd: Option<&Path>, excluded: &[PathBuf]) -> Discovery {
    if path.is_dir() {
        walk(path, cwd, excluded)
    } else {
        Discovery {
            files: named_file(path),
            errors: Vec::new(),
        }
    }
}

fn named_file(path: &Path) -> Vec<PathBuf> {
    if path.is_file() && is_python(path) {
        vec![path.to_path_buf()]
    } else {
        Vec::new()
    }
}

/// Walks `dir`, which was named on the command line. When `dir` is itself an excluded path, or lies inside
/// one, nothing is excluded below it: the walk checks everything, as it does for a file named directly.
fn walk(dir: &Path, cwd: Option<&Path>, excluded: &[PathBuf]) -> Discovery {
    let excluded = excluded_below(&normalize(dir, cwd), excluded);
    let mut found = Discovery::default();
    for entry in WalkBuilder::new(dir).require_git(false).build() {
        match entry {
            Ok(entry) => found.files.extend(python_file(entry, cwd, excluded)),
            Err(error) => found.errors.push(walk_error(dir, &error)),
        }
    }
    found
}

fn excluded_below<'a>(root: &Path, excluded: &'a [PathBuf]) -> &'a [PathBuf] {
    if excluded.iter().any(|entry| root.starts_with(entry)) {
        &[]
    } else {
        excluded
    }
}

fn python_file(entry: DirEntry, cwd: Option<&Path>, excluded: &[PathBuf]) -> Option<PathBuf> {
    let kind = entry.file_type()?;
    let file = entry.into_path();
    let wanted =
        is_python(&file) && !is_excluded(&file, cwd, excluded) && is_checked_entry(&file, kind);
    wanted.then_some(file)
}

fn is_checked_entry(file: &Path, kind: fs::FileType) -> bool {
    if kind.is_symlink() {
        is_checked_link(file)
    } else {
        kind.is_file()
    }
}

/// Whether a symbolic link that the walk found is checked. It is when it points to a regular file. It is
/// also when it points to nothing, or cannot be followed, so that presence reports it as GW000 and does
/// not skip it silently. A link to a directory is not checked, and the walk does not descend into it.
fn is_checked_link(link: &Path) -> bool {
    match fs::metadata(link) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => true,
    }
}

/// The walk names the path in the error when it can, and otherwise the directory that was being walked is reported.
fn walk_error(root: &Path, error: &ignore::Error) -> Diagnostic {
    let path = match error {
        ignore::Error::WithPath { path, .. } => path.as_path(),
        _ => root,
    };
    let what = if path.is_dir() { "directory" } else { "path" };
    let detail = cause_text(error);
    Diagnostic::for_path(path, format!("cannot read {what}: {detail}"))
}

/// The operating system error behind a walk error, such as "Permission denied (os error 13)". The walk
/// wraps that error in one that also names the path, and the diagnostic already gives the path.
fn cause_text(error: &ignore::Error) -> String {
    let cause = error
        .io_error()
        .and_then(std::io::Error::get_ref)
        .and_then(std::error::Error::source);
    cause.map_or_else(|| error.to_string(), ToString::to_string)
}

fn is_python(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "py")
}

fn is_excluded(file: &Path, cwd: Option<&Path>, excluded: &[PathBuf]) -> bool {
    let relative = normalize(file, cwd);
    excluded.iter().any(|entry| relative.starts_with(entry))
}

/// Makes an absolute path relative to `cwd` when it lies under it, and drops `.` components so
/// that `./tests/fixtures` and `tests/fixtures` compare equal.
fn normalize(path: &Path, cwd: Option<&Path>) -> PathBuf {
    let relative = match cwd {
        Some(cwd) if path.is_absolute() => path.strip_prefix(cwd).unwrap_or(path),
        _ => path,
    };
    relative
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use super::walk_error;

    #[test]
    fn walk_error_without_a_path_is_reported_at_the_directory_walked() {
        let error = ignore::Error::Io(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        let diagnostic = walk_error(Path::new("no-such-directory-for-gawain"), &error);

        assert_eq!(diagnostic.path, Path::new("no-such-directory-for-gawain"));
        assert_eq!(diagnostic.message, "cannot read path: denied");
    }
}
