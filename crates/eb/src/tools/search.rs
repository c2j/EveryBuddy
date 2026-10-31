//! Search tools, built on the ripgrep library family (pinned at the
//! workspace root): `ignore` walks the tree, `grep-regex`/`grep-searcher`
//! match and stream lines, `globset` matches file names.

use std::path::Path;

use globset::GlobBuilder;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, Sink, SinkMatch};
use ignore::WalkBuilder;

use super::*;

/// Most matched lines reported for one search. The model can narrow the
/// pattern or path instead of paging through noise.
const MATCH_LINE_CAP: usize = 2_000;
/// Maximum entries for one `search_file`.
const PATH_LINE_CAP: usize = 1_000;

/// One shared walk configuration: include hidden files (models often need
/// dotfiles), skip `.git` internals, honor `.gitignore` only when asked.
/// Defaulting gitignore off is deliberate — an agent looking for build
/// artifacts usually *wants* the ignored tree.
fn walker(root: &Path, respect_gitignore: bool) -> WalkBuilder {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false); // WalkBuilder.hidden(true) SKIPS hidden entries.
    walk.git_ignore(respect_gitignore);
    walk.filter_entry(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| name != ".git")
            .unwrap_or(true)
    });
    walk
}

/// `search_content(pattern, path?, regex?, ignore_case?, respect_gitignore?)`
/// — search file contents, reporting `path:line: text`.
pub struct SearchContent;

/// Line sink that formats `path:line` and stops after the cap.
struct ContentSink<'a> {
    path: &'a str,
    hits: usize,
    out: &'a mut String,
    truncated: bool,
}

impl Sink for ContentSink<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        if self.hits >= MATCH_LINE_CAP {
            self.truncated = true;
            return Ok(false); // stop searching this file
        }
        self.hits += 1;
        let text = String::from_utf8_lossy(mat.bytes());
        let text = text.trim_end();
        let text = if text.len() > 300 { &text[..300] } else { text };
        self.out.push_str(&format!("{}:{}: {}\n", self.path, mat.line_number().unwrap_or(0), text));
        Ok(true)
    }
}

impl BuiltinTool for SearchContent {
    fn name(&self) -> &'static str {
        "search_content"
    }

    fn description(&self) -> &'static str {
        "Search file contents under a directory (default: the workspace root) \
and report `path:line: text` for every match. Binary and non-UTF-8 files are \
detected and skipped automatically. Prefer a specific path over the whole \
workspace."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "pattern": { "type": "string", "description": "Text or regular expression to find" },
            "path": { "type": "string", "description": "Directory (or single file) to search; default '.'" },
            "regex": { "type": "boolean", "description": "Treat pattern as a regular expression (default true); false = literal text" },
            "ignore_case": { "type": "boolean", "description": "Case-insensitive matching (default false)" },
            "respect_gitignore": { "type": "boolean", "description": "Skip files listed in .gitignore (default false)" }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let pattern = args["pattern"].as_str().unwrap_or_default();
        let raw = args["path"].as_str().unwrap_or(".").to_string();
        if pattern.is_empty() {
            return report(ctx, "Failed", "Search".to_string(), "error: pattern is empty".into());
        }
        let respect = args["respect_gitignore"].as_bool().unwrap_or(false);
        let root = match resolve_in(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("Search {raw}"), error);
            }
        };
        let mut builder = RegexMatcherBuilder::new();
        builder.case_insensitive(args["ignore_case"].as_bool().unwrap_or(false));
        // Literal mode: quote the pattern instead of trusting a builder flag,
        // so behavior is identical across grep-regex versions.
        let effective = if args["regex"].as_bool().unwrap_or(true) {
            pattern.to_string()
        } else {
            regex_escape(pattern)
        };
        let matcher = match builder.build(&effective) {
            Ok(matcher) => matcher,
            Err(error) => {
                return report(
                    ctx,
                    "Failed",
                    format!("Search {raw}"),
                    format!("invalid pattern: {error}"),
                );
            }
        };
        // Stop at a NUL byte, like rg: binary files are skipped, not
        // dumped into the transcript as replacement chars.
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .build();

        let mut out = String::new();
        let mut truncated = false;
        let mut files_searched = 0usize;
        for entry in walker(&root, respect).build() {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            files_searched += 1;
            if out.len() >= TOOL_OUTPUT_CAP {
                truncated = true;
                break;
            }
            let display = entry
                .path()
                .strip_prefix(&root)
                .map(|rest| {
                    if rest.as_os_str().is_empty() {
                        raw.clone()
                    } else {
                        format!("{raw}/{}", rest.display())
                    }
                })
                .unwrap_or_else(|_| entry.path().display().to_string());
            let mut sink = ContentSink {
                path: &display,
                hits: 0,
                out: &mut out,
                truncated: false,
            };
            let _ = searcher.search_path(&matcher, entry.path(), &mut sink);
            if sink.truncated {
                truncated = true;
            }
        }
        let summary = if out.is_empty() {
            format!(
                "no matches for {pattern:?} (searched {files_searched} files)"
            )
        } else {
            if truncated {
                out.push_str(&format!(
                    "[stopped at {} lines or the output cap]\n",
                    MATCH_LINE_CAP
                ));
            }
            out.push_str(&format!(
                "({files_searched} files searched; cap {MATCH_LINE_CAP} matches)"
            ));
            out
        };
        report(
            ctx,
            "Ran",
            format!("Search {pattern:?} in {raw}"),
            truncate_tail(&summary, TOOL_OUTPUT_CAP),
        )
    }
}

/// `search_file(pattern, path?, respect_gitignore?)` — glob file names.
pub struct SearchFile;

impl BuiltinTool for SearchFile {
    fn name(&self) -> &'static str {
        "search_file"
    }

    fn description(&self) -> &'static str {
        "Find files by glob pattern (e.g. `**/*.rs`, `src/*.toml`), relative \
to the given directory (default the workspace root). Directories are not \
returned; use list_dir for a plain listing."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "pattern": { "type": "string", "description": "Glob such as **/*.rs or src/*.toml" },
            "path": { "type": "string", "description": "Directory to search; default '.'" },
            "respect_gitignore": { "type": "boolean", "description": "Skip files listed in .gitignore (default false)" }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let pattern = args["pattern"].as_str().unwrap_or_default();
        let raw = args["path"].as_str().unwrap_or(".").to_string();
        let respect = args["respect_gitignore"].as_bool().unwrap_or(false);
        let root = match resolve_in(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("Find {pattern}"), error);
            }
        };
        let glob = match GlobBuilder::new(pattern)
            .literal_separator(true) // `*` must not cross `/`
            .build()
        {
            Ok(glob) => glob,
            Err(error) => {
                return report(
                    ctx,
                    "Failed",
                    format!("Find {pattern}"),
                    format!("invalid glob: {error}"),
                );
            }
        };
        let matcher = globset::GlobSetBuilder::new()
            .add(glob)
            .build()
            .expect("one glob always builds a set");

        let mut out = String::new();
        let mut count = 0usize;
        let mut truncated = false;
        for entry in walker(&root, respect).build() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            // Match both the whole path and the suffix after the search root,
            // so `src/*.rs` works whether pattern is rooted or not.
            let relative = path.strip_prefix(&root).unwrap_or(path);
            if !matcher.is_match(relative) && !matcher.is_match(path) {
                continue;
            }
            count += 1;
            if count > PATH_LINE_CAP {
                truncated = true;
                break;
            }
            out.push_str(&relative.display().to_string());
            out.push('\n');
        }
        let summary = if out.is_empty() {
            format!("no files match {pattern:?}")
        } else {
            if truncated {
                out.push_str(&format!("[stopped at {PATH_LINE_CAP} files]\n"));
            }
            out
        };
        report(
            ctx,
            "Ran",
            format!("Find {pattern} in {raw}"),
            truncate_tail(&summary, TOOL_OUTPUT_CAP),
        )
    }
}
