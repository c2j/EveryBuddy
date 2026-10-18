//! The capability that justifies a Rust core: multi-pattern content search,
//! in process.
//!
//! dsh's own search tools shell out to a packaged `ripgrep` binary through the
//! subprocess seam. That is unavailable wherever spawning is forbidden, costs a
//! process per call, and cannot be composed. `aho-corasick` plus `memchr` gives
//! the same class of matcher inside the core, with no subprocess and no
//! privileged operation beyond the preopens the host already granted.
use std::path::{Path, PathBuf};

use aho_corasick::AhoCorasick;
use serde::Serialize;

use crate::walk::Walker;

/// Bounds every scan so one call cannot exhaust memory or the answer budget.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct SearchLimits {
    /// Files larger than this are counted and skipped.
    pub max_file_bytes: u64,
    /// Stop collecting matches after this many, and mark the outcome truncated.
    pub max_matches: usize,
    /// Stop walking after this many regular files.
    pub max_files: usize,
    /// Longest snippet of the matching line kept in a report.
    pub snippet_chars: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 2 * 1024 * 1024,
            max_matches: 500,
            max_files: 20_000,
            snippet_chars: 200,
        }
    }
}

/// One occurrence of one pattern.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column, counted in bytes from the line start.
    pub column: usize,
    /// Index into the pattern list the scan was built with.
    pub pattern_index: usize,
    pub pattern: String,
    /// The matching line, trimmed and capped at `snippet_chars`.
    pub text: String,
}

/// What one scan over one root produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub root: String,
    pub patterns: Vec<String>,
    pub hits: Vec<Hit>,
    /// Matches found, including those past `max_matches`.
    pub total_matches: usize,
    pub files_scanned: usize,
    /// Files counted but not read: binary, too large, or unreadable.
    pub files_skipped: usize,
    pub directories_skipped: usize,
    pub bytes_scanned: u64,
    pub truncated: bool,
    /// Inclusive scan duration. Requires a clock, so the core is built for
    /// `wasm32-wasip1`.
    pub elapsed_ms: f64,
}

/// A compiled multi-pattern matcher.
///
/// The matcher is built once and reused across roots, which is the whole reason
/// it is a service rather than a function.
#[derive(Debug)]
pub struct SearchEngine {
    patterns: Vec<String>,
    matcher: AhoCorasick,
    limits: SearchLimits,
}

impl SearchEngine {
    /// Compile `patterns`. An empty list is rejected: a scan that cannot match
    /// anything would look like a successful empty result.
    pub fn new(patterns: Vec<String>, limits: SearchLimits) -> Result<Self, String> {
        if patterns.is_empty() {
            return Err("no patterns given".to_string());
        }
        let matcher = AhoCorasick::new(&patterns).map_err(|e| format!("pattern compile failed: {e}"))?;
        Ok(Self {
            patterns,
            matcher,
            limits,
        })
    }

    /// The pattern list this engine was compiled with.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// The bounds in force.
    pub fn limits(&self) -> &SearchLimits {
        &self.limits
    }

    /// Scan `root`, returning matches in deterministic file order.
    pub fn find(&self, root: &Path) -> Result<SearchOutcome, String> {
        let started = std::time::Instant::now();
        let walk = Walker::new(root, self.limits.max_files).collect();

        let mut hits = Vec::new();
        let mut total_matches = 0usize;
        let mut files_scanned = 0usize;
        let mut files_skipped = 0usize;
        let mut bytes_scanned = 0u64;
        let mut truncated = walk.truncated;

        for path in &walk.files {
            if hits.len() >= self.limits.max_matches {
                truncated = true;
                break;
            }

            let bytes = match read_within(path, self.limits.max_file_bytes) {
                Some(bytes) => bytes,
                None => {
                    files_skipped += 1;
                    continue;
                }
            };
            if looks_binary(&bytes) {
                files_skipped += 1;
                continue;
            }

            files_scanned += 1;
            bytes_scanned += bytes.len() as u64;

            let mut line_starts: Option<Vec<usize>> = None;
            for found in self.matcher.find_iter(&bytes) {
                total_matches += 1;
                if hits.len() >= self.limits.max_matches {
                    truncated = true;
                    break;
                }
                let starts = line_starts.get_or_insert_with(|| line_starts_of(&bytes));
                let line = line_of(starts, found.start());
                let line_end = line_end_of(&bytes, found.start());
                hits.push(Hit {
                    path: display_path(path, root),
                    line: line + 1,
                    column: found.start() - starts[line] + 1,
                    pattern_index: found.pattern().as_usize(),
                    pattern: self.patterns[found.pattern().as_usize()].clone(),
                    text: snippet(&bytes[starts[line]..line_end], self.limits.snippet_chars),
                });
            }
        }

        Ok(SearchOutcome {
            root: root.display().to_string(),
            patterns: self.patterns.clone(),
            hits,
            total_matches,
            files_scanned,
            files_skipped,
            directories_skipped: walk.dirs_skipped,
            bytes_scanned,
            truncated,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// Read a file only if it is within `max_bytes`, so an oversized file cannot be
/// pulled into wasm memory.
fn read_within(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    std::fs::read(path).ok()
}

/// A NUL byte in the head of a file is the same heuristic ripgrep uses to call
/// a file binary.
fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    memchr::memchr(0, head).is_some()
}

/// Byte offset of the first character of each line.
fn line_starts_of(bytes: &[u8]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(bytes.len() / 40 + 1);
    starts.push(0);
    let mut at = 0;
    while let Some(found) = memchr::memchr(b'\n', &bytes[at..]) {
        at += found + 1;
        starts.push(at);
    }
    starts
}

/// Index of the line containing `offset`.
fn line_of(starts: &[usize], offset: usize) -> usize {
    match starts.binary_search(&offset) {
        Ok(index) => index,
        // `starts` always begins with 0, so an insert position of 0 is
        // unreachable for a valid offset.
        Err(index) => index - 1,
    }
}

/// Byte offset just past the end of the line containing `offset`.
fn line_end_of(bytes: &[u8], offset: usize) -> usize {
    let rest = &bytes[offset..];
    offset + memchr::memchr(b'\n', rest).unwrap_or(rest.len())
}

/// Trim one line and cap it, keeping the result valid UTF-8.
fn snippet(line: &[u8], max_chars: usize) -> String {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// A path relative to the scan root when possible, so reports stay readable.
fn display_path(path: &PathBuf, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evobuddy-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn engine(patterns: &[&str]) -> SearchEngine {
        SearchEngine::new(
            patterns.iter().map(|s| s.to_string()).collect(),
            SearchLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn reports_line_and_column() {
        let root = scratch("pos");
        std::fs::write(root.join("a.txt"), "alpha\nbeta gamma\n").unwrap();

        let outcome = engine(&["gamma"]).find(&root).unwrap();

        assert_eq!(outcome.hits.len(), 1);
        let hit = &outcome.hits[0];
        assert_eq!(hit.path, "a.txt");
        assert_eq!(hit.line, 2);
        assert_eq!(hit.column, 6);
        assert_eq!(hit.text, "beta gamma");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn one_pass_finds_every_pattern() {
        let root = scratch("multi");
        std::fs::write(root.join("a.txt"), "fn a()\npub fn b()\nasync fn c()\n").unwrap();

        let outcome = engine(&["fn ", "pub ", "async "]).find(&root).unwrap();

        // `fn ` is matched inside both `pub fn` and `async fn`, so one pass over
        // one file yields five hits in source order.
        assert_eq!(outcome.total_matches, 5);
        let patterns: Vec<usize> = outcome.hits.iter().map(|h| h.pattern_index).collect();
        assert_eq!(patterns, vec![0, 1, 0, 2, 0]);
        let lines: Vec<usize> = outcome.hits.iter().map(|h| h.line).collect();
        assert_eq!(lines, vec![1, 2, 2, 3, 3]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn binary_files_are_skipped_not_reported() {
        let root = scratch("binary");
        std::fs::write(root.join("a.bin"), b"\x00\x01needle\x00").unwrap();
        std::fs::write(root.join("b.txt"), "needle\n").unwrap();

        let outcome = engine(&["needle"]).find(&root).unwrap();

        assert_eq!(outcome.files_scanned, 1);
        assert_eq!(outcome.files_skipped, 1);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].path, "b.txt");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn oversized_files_are_counted_and_left_unread() {
        let root = scratch("large");
        std::fs::write(root.join("big.txt"), "needle\n".repeat(50)).unwrap();

        let mut limits = SearchLimits::default();
        limits.max_file_bytes = 16;
        let engine = SearchEngine::new(vec!["needle".to_string()], limits).unwrap();
        let outcome = engine.find(&root).unwrap();

        assert_eq!(outcome.files_scanned, 0);
        assert_eq!(outcome.files_skipped, 1);
        assert!(outcome.hits.is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn match_limit_truncates_instead_of_growing_without_bound() {
        let root = scratch("limit");
        std::fs::write(root.join("a.txt"), "x\n".repeat(100)).unwrap();

        let mut limits = SearchLimits::default();
        limits.max_matches = 10;
        let engine = SearchEngine::new(vec!["x".to_string()], limits).unwrap();
        let outcome = engine.find(&root).unwrap();

        assert_eq!(outcome.hits.len(), 10);
        assert!(outcome.truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn empty_pattern_list_is_rejected() {
        let err = SearchEngine::new(Vec::new(), SearchLimits::default()).unwrap_err();
        assert!(err.contains("no patterns"));
    }

    #[test]
    fn snippets_are_capped_and_keep_the_summary_line() {
        let root = scratch("snippet");
        std::fs::write(root.join("a.txt"), format!("needle{}\n", "z".repeat(500))).unwrap();

        let mut limits = SearchLimits::default();
        limits.snippet_chars = 20;
        let engine = SearchEngine::new(vec!["needle".to_string()], limits).unwrap();
        let outcome = engine.find(&root).unwrap();

        let text = &outcome.hits[0].text;
        assert_eq!(text.chars().count(), 21, "20 chars plus the ellipsis");
        assert!(text.ends_with('…'));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
