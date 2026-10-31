//! Mutating file tools: write, precise edit, delete. Each goes through the
//! permission gate declared by its class.

use super::*;

/// `write_file(path, content)` — create or overwrite a UTF-8 file.
pub struct WriteFile;

impl BuiltinTool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create or overwrite a UTF-8 text file (parents are created). Requires \
permission. For small edits inside an existing file prefer replace_in_file."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "path": { "type": "string", "description": "File path" },
            "content": { "type": "string", "description": "Full file content" }
        })
    }

    fn class(&self) -> PermClass {
        PermClass::Write
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let raw = args["path"].as_str().unwrap_or_default().to_string();
        let content = args["content"].as_str().unwrap_or_default();
        let path = match resolve_in(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("Write {raw}"), error);
            }
        };
        let write = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .and_then(|_| std::fs::write(&path, content));
        match write {
            Ok(()) => report(
                ctx,
                "Ran",
                format!("Write {raw}"),
                format!("wrote {} bytes to {raw}", content.len()),
            ),
            Err(error) => report(
                ctx,
                "Failed",
                format!("Write {raw}"),
                format!("write failed: {error}"),
            ),
        }
    }
}

/// `replace_in_file(path, old_string, new_string)` — one precise, literal
/// replacement. Deliberately no regex: the model writes the pattern, and a
/// regex would turn a stray `(` into a confusing failure. Zero or multiple
/// matches are errors on purpose, so the model widens its context instead of
/// editing the wrong place.
pub struct ReplaceInFile;

impl BuiltinTool for ReplaceInFile {
    fn name(&self) -> &'static str {
        "replace_in_file"
    }

    fn description(&self) -> &'static str {
        "Replace one exact occurrence of old_string with new_string in a \
UTF-8 text file. The match must be unique: 0 matches or several matches are \
errors, so include enough surrounding lines to disambiguate. Requires \
permission."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "path": { "type": "string", "description": "File path" },
            "old_string": { "type": "string", "description": "Exact text to replace (must occur exactly once)" },
            "new_string": { "type": "string", "description": "Replacement text" }
        })
    }

    fn class(&self) -> PermClass {
        PermClass::Write
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let raw = args["path"].as_str().unwrap_or_default().to_string();
        let old = args["old_string"].as_str().unwrap_or_default();
        let new = args["new_string"].as_str().unwrap_or_default();
        let path = match resolve_in(ctx.root, &raw) {
            Ok(path) => path,
            Err(error) => {
                return report(ctx, "Failed", format!("Edit {raw}"), error);
            }
        };
        if old.is_empty() {
            return report(
                ctx,
                "Failed",
                format!("Edit {raw}"),
                "error: old_string is empty".to_string(),
            );
        }
        let edit = std::fs::read_to_string(&path).and_then(|content| {
            let hits = content.matches(old).count();
            match hits {
                0 => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "old_string not found in file; re-read the file and include \
more surrounding context to make the match unique",
                )),
                1 => {
                    std::fs::write(&path, content.replacen(old, new, 1))?;
                    Ok(())
                }
                n => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "old_string matches {n} places; include more surrounding \
context to make it unique"
                    ),
                )),
            }
        });
        match edit {
            Ok(()) => {
                let added = new.lines().count();
                let removed = old.lines().count();
                let _ = ctx.sink.send(eb_api::AgentEvent::Block {
                    block: eb_api::model::Block::Diff {
                        path: raw.clone(),
                        added: added as u32,
                        removed: removed as u32,
                        lines: vec![],
                    },
                });
                report(
                    ctx,
                    "Ran",
                    format!("Edit {raw}"),
                    format!("replaced {removed} line(s) with {added} line(s) in {raw}"),
                )
            }
            Err(error) => report(
                ctx,
                "Failed",
                format!("Edit {raw}"),
                format!("replace failed: {error}"),
            ),
        }
    }
}

/// `delete_files(paths[])` — delete files or directories inside the
/// workspace. Its own permission class: deleting is a step beyond writing.
pub struct DeleteFiles;

impl BuiltinTool for DeleteFiles {
    fn name(&self) -> &'static str {
        "delete_files"
    }

    fn description(&self) -> &'static str {
        "Delete files or directories (recursively) inside the task workspace. \
Requires permission; deleting is gated separately from writing."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "paths": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Paths to delete, relative to the workspace"
            }
        })
    }

    fn class(&self) -> PermClass {
        PermClass::Delete
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let Some(paths) = args["paths"].as_array() else {
            return report(
                ctx,
                "Failed",
                "Delete".to_string(),
                "error: paths must be an array of strings".to_string(),
            );
        };
        let mut results: Vec<String> = Vec::new();
        let mut any_failed = false;
        for value in paths {
            let Some(raw) = value.as_str() else {
                continue;
            };
            let outcome = resolve_in(ctx.root, raw)
                .map_err(|error| error)
                .and_then(|path| {
                    // Compare canonicalized: on macOS /var is a symlink to
                    // /private/var, so a textual equality check can miss.
                    let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
                    let root_canonical =
                        std::fs::canonicalize(ctx.root).unwrap_or_else(|_| ctx.root.to_path_buf());
                    if canonical == root_canonical {
                        Err("refusing to delete the workspace root".to_string())
                    } else if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                            .map(|_| format!("deleted directory {raw}"))
                            .map_err(|error| format!("delete failed: {error}"))
                    } else {
                        std::fs::remove_file(&path)
                            .map(|_| format!("deleted {raw}"))
                            .map_err(|error| format!("delete failed: {error}"))
                    }
                });
            match outcome {
                Ok(message) => results.push(message),
                Err(message) => {
                    any_failed = true;
                    results.push(format!("{raw}: {message}"));
                }
            }
        }
        let joined = results.join("\n");
        report(
            ctx,
            if any_failed { "Failed" } else { "Ran" },
            format!("Delete {}", gate_title(args)),
            truncate_tail(&joined, TOOL_OUTPUT_CAP),
        )
    }
}
