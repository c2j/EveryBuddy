//! Read-only file tools: never prompt.

use super::*;

/// `read_file(path, offset?, limit?)` — read a UTF-8 text file, optionally
/// one page of lines. Long output is tail-trimmed, and the reply reports the
/// total line count so the model can page deliberately instead of guessing.
pub struct ReadFile;

impl BuiltinTool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a UTF-8 text file. Relative paths resolve against the task \
workspace. Pass offset (1-based first line) and limit to page through large \
files; the result reports the total line count."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "path": { "type": "string", "description": "File path" },
            "offset": { "type": "integer", "description": "1-based first line to read (default 1)" },
            "limit": { "type": "integer", "description": "Number of lines to read (default: to end, capped)" }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let raw = args["path"].as_str().unwrap_or_default();
        let path = match resolve_in(ctx.root, raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("Read {raw}"), error);
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let total = content.lines().count();
                let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize - 1;
                let limit = args["limit"]
                    .as_u64()
                    .map(|limit| limit as usize)
                    // Cap a full read at a sane number of lines; the model can
                    // page with offset instead of re-reading everything.
                    .unwrap_or(2_000);
                let selected: Vec<&str> = content
                    .lines()
                    .skip(offset)
                    .take(limit)
                    .collect();
                let mut body = selected.join("\n");
                if body.len() > TOOL_OUTPUT_CAP {
                    body = truncate_tail(&body, TOOL_OUTPUT_CAP);
                }
                let header = if offset == 0 && total <= limit {
                    format!("({total} lines)")
                } else {
                    format!(
                        "(lines {}-{} of {total})",
                        offset + 1,
                        offset + selected.len()
                    )
                };
                report(
                    ctx,
                    "Ran",
                    format!("Read {raw}"),
                    if body.is_empty() {
                        format!("{header} (empty range)")
                    } else {
                        format!("{header}\n{body}")
                    },
                )
            }
            Err(error) => report(
                ctx,
                "Failed",
                format!("Read {raw}"),
                format!("read failed: {error}"),
            ),
        }
    }
}

/// `list_dir(path)` — list one directory's entries.
pub struct ListDir;

impl BuiltinTool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List a directory's entries. Directories carry a trailing slash."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "path": { "type": "string", "description": "Directory path, '.' for the workspace root" }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let raw = args["path"].as_str().unwrap_or(".").to_string();
        let path = match resolve_in(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("List {raw}"), error);
            }
        };
        let mut entries: Vec<String> = Vec::new();
        match std::fs::read_dir(&path) {
            Ok(read) => {
                for entry in read.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let mark = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        "/"
                    } else {
                        ""
                    };
                    entries.push(format!("{name}{mark}"));
                }
                entries.sort();
                let listing = entries.join("\n");
                report(
                    ctx,
                    "Ran",
                    format!("List {raw}"),
                    truncate_tail(&listing, TOOL_OUTPUT_CAP),
                )
            }
            Err(error) => report(
                ctx,
                "Failed",
                format!("List {raw}"),
                format!("list failed: {error}"),
            ),
        }
    }
}
