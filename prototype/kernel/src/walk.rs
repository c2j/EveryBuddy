//! Directory traversal for the search engine.
//!
//! Rust does the traversal itself through `std::fs`, which reaches the real
//! filesystem through WASI preopens. The alternative — asking the host to list
//! each directory — would put a call boundary in the middle of every tree walk
//! for no benefit.
//!
//! Order is deterministic (entries sorted per directory) so that a given tree
//! produces the same result list on every run.
use std::path::{Path, PathBuf};

/// Directory names never descended into. These are build outputs and dependency
/// trees: searching them wastes the whole budget on generated content.
pub const DEFAULT_SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
];

/// What one traversal produced.
#[derive(Debug, Default)]
pub struct WalkOutcome {
    /// Regular files to search, in deterministic order.
    pub files: Vec<PathBuf>,
    /// Directories not descended into because of [`DEFAULT_SKIP_DIRS`].
    pub dirs_skipped: usize,
    /// Directory entries that could not be read (permission, races).
    pub dirs_unreadable: usize,
    /// True when the traversal bound or `max_files` cut the walk short.
    pub truncated: bool,
}

/// Hard bound on directory entries visited, independent of `max_files`: the
/// result bound must not turn into an unbounded traversal on a pathological
/// tree. Far above any real repository; reaching it reports truncation.
const MAX_TRAVERSAL_ENTRIES: usize = 500_000;

/// Breadth-first walker with a bounded result set.
///
/// Recursion is avoided deliberately: a deep tree must not be able to overflow
/// the wasm stack, which is much smaller than a native one.
pub struct Walker<'a> {
    root: &'a Path,
    skip_dirs: &'a [&'a str],
    max_files: usize,
}

impl<'a> Walker<'a> {
    /// Bound the walk to `max_files` regular files under `root`.
    pub fn new(root: &'a Path, max_files: usize) -> Self {
        Self {
            root,
            skip_dirs: DEFAULT_SKIP_DIRS,
            max_files,
        }
    }

    /// Replace the skip list. Names are matched exactly, not as globs.
    pub fn with_skip_dirs(mut self, skip_dirs: &'a [&'a str]) -> Self {
        self.skip_dirs = skip_dirs;
        self
    }

    /// Walk the tree, returning the files found and what was left out.
    ///
    /// The result is **globally sorted by path, then truncated to `max_files`**,
    /// so a truncated walk covers the alphabetically first files rather than an
    /// arbitrary traversal prefix. An early return at the cap would be cheaper
    /// but non-deterministic — exactly the bug this ordering prevents.
    pub fn collect(&self) -> WalkOutcome {
        let mut outcome = WalkOutcome::default();
        let mut pending = vec![self.root.to_path_buf()];
        let mut visited = 0usize;

        while let Some(dir) = pending.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => {
                    outcome.dirs_unreadable += 1;
                    continue;
                }
            };

            for entry in entries.flatten() {
                visited += 1;
                if visited > MAX_TRAVERSAL_ENTRIES {
                    outcome.truncated = true;
                    break;
                }
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // `file_type` avoids a second stat on platforms where the
                // directory entry already carries the type.
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    if self.skip_dirs.iter().any(|skip| *skip == name) {
                        outcome.dirs_skipped += 1;
                    } else {
                        pending.push(path);
                    }
                } else {
                    outcome.files.push(path);
                }
            }
            if outcome.truncated {
                break;
            }
        }

        outcome.files.sort();
        if outcome.files.len() > self.max_files {
            outcome.files.truncate(self.max_files);
            outcome.truncated = true;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evobuddy-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_files_and_skips_build_dirs() {
        let root = scratch("skip");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        std::fs::write(root.join("node_modules/pkg/b.rs"), "fn b() {}").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        let outcome = Walker::new(&root, 100).collect();

        let names: Vec<String> = outcome
            .files
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Cargo.toml", "src/a.rs"]);
        assert_eq!(outcome.dirs_skipped, 1);
        assert!(!outcome.truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reports_truncation_at_the_file_limit() {
        let root = scratch("limit");
        for i in 0..5 {
            std::fs::write(root.join(format!("f{i}.txt")), "x").unwrap();
        }
        let outcome = Walker::new(&root, 3).collect();
        assert_eq!(outcome.files.len(), 3);
        assert!(outcome.truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn order_is_deterministic() {
        let root = scratch("order");
        for name in ["c.txt", "a.txt", "b.txt"] {
            std::fs::write(root.join(name), "x").unwrap();
        }
        let first = Walker::new(&root, 10).collect().files;
        let second = Walker::new(&root, 10).collect().files;
        assert_eq!(first, second);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
