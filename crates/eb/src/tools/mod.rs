//! The Custom Model agent's built-in toolset.
//!
//! Each tool is a small unit struct implementing [`BuiltinTool`]: an OpenAI
//! function schema plus one `run`. The registry in this module generates the
//! `tools` array for the request body and dispatches execution, applying the
//! permission gate in one place. WorkBuddy parity drove the tool list; the
//! search tools sit on the ripgrep library family (pinned at the workspace
//! root).

pub mod exec;
pub mod fs_edit;
pub mod fs_read;
pub mod meta;
pub mod search;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eb_api::{AgentSink, PermissionDecision};

use crate::router;
use exec::JobTable;

/// Permission class of one tool: decides whether a call prompts the user.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermClass {
    /// Pure read; never prompts.
    None,
    Write,
    Delete,
    Command,
}

impl PermClass {
    fn consent_flag(self, consent: &ToolConsent) -> bool {
        match self {
            PermClass::None => true,
            PermClass::Write => consent.allow_writes_all,
            PermClass::Delete => consent.allow_deletes_all,
            PermClass::Command => consent.allow_commands_all,
        }
    }

    fn remember(self, consent: &mut ToolConsent) {
        match self {
            PermClass::None => {}
            PermClass::Write => consent.allow_writes_all = true,
            PermClass::Delete => consent.allow_deletes_all = true,
            PermClass::Command => consent.allow_commands_all = true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            PermClass::None => "",
            PermClass::Write => "write",
            PermClass::Delete => "delete",
            PermClass::Command => "command",
        }
    }
}

/// Turn-scoped consent memory: an "Always allow" answer covers the rest of
/// the turn for that tool class, so a multi-step task does not re-prompt on
/// every call. Scoped to the turn (not the process) on purpose — each new
/// task re-earns trust.
#[derive(Default)]
pub struct ToolConsent {
    pub allow_writes_all: bool,
    pub allow_deletes_all: bool,
    pub allow_commands_all: bool,
}

/// One accumulated tool call from the SSE stream, in arrival (index) order.
#[derive(Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Everything a tool needs at run time. Built per call by [`execute_tool`].
pub struct ToolContext<'a> {
    pub root: &'a Path,
    pub sink: &'a AgentSink,
    /// Background command jobs started by `run_command` in this turn; they
    /// must outlive one round for `command_output`/`kill` to reach them.
    pub jobs: &'a mut JobTable,
}

type AskFn<'a> = &'a mut dyn FnMut(&str, &str) -> PermissionDecision;

/// One built-in tool: schema plus behavior. Implementations emit their own
/// `Block::Tool` rows and return the tool-result text fed back to the model.
pub trait BuiltinTool: Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON-schema `properties`/`required` for the function parameters.
    fn parameters(&self) -> serde_json::Value;
    fn class(&self) -> PermClass {
        PermClass::None
    }
    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, ask: AskFn) -> String;
}

/// Every registered tool, in a stable order (the request's `tools` array and
/// the model's behavior both benefit from stable ordering).
pub fn registry() -> Vec<&'static dyn BuiltinTool> {
    vec![
        &fs_read::ReadFile,
        &fs_read::ListDir,
        &fs_edit::WriteFile,
        &fs_edit::ReplaceInFile,
        &fs_edit::DeleteFiles,
        &search::SearchContent,
        &search::SearchFile,
        &exec::RunCommand,
        &exec::CommandOutput,
        &exec::KillCommand,
        &meta::TodoWrite,
    ]
}

/// The `tools` array for an OpenAI-compatible request body.
pub fn tool_definitions() -> serde_json::Value {
    serde_json::json!(registry()
        .iter()
        .map(|tool| serde_json::json!({
            "type": "function",
            "function": {
                "name": tool.name(),
                "description": tool.description(),
                "parameters": {
                    "type": "object",
                    "properties": tool.parameters(),
                    "required": required_of(tool.name()),
                }
            }
        }))
        .collect::<Vec<_>>())
}

/// Required-parameter lists, kept out of each tool so the schema objects stay
/// one-liners. A tool with no required parameters lists none.
fn required_of(name: &str) -> serde_json::Value {
    let names: &[&[&str]] = &[
        &["read_file", "path"],
        &["list_dir", "path"],
        &["write_file", "path", "content"],
        &["replace_in_file", "path", "old_string", "new_string"],
        &["delete_files", "paths"],
        &["search_content", "pattern"],
        &["search_file", "pattern"],
        &["run_command", "command"],
        &["command_output", "id"],
        &["kill_command", "id"],
        &["todo_write", "todos"],
    ];
    let required = names
        .iter()
        .find(|entry| entry[0] == name)
        .map(|entry| &entry[1..])
        .unwrap_or(&[]);
    serde_json::json!(required)
}

/// Execute one call: parse arguments, gate on permission class, dispatch.
/// This is the single funnel the streaming round loop calls per tool call.
pub fn execute_tool(
    call: &ToolCall,
    root: &Path,
    consent: &mut ToolConsent,
    ask: AskFn,
    sink: &AgentSink,
    jobs: &mut JobTable,
) -> String {
    let tool = match registry().into_iter().find(|tool| tool.name() == call.name)
    {
        Some(tool) => tool,
        None => return format!("error: unknown tool {}", call.name),
    };
    let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
        Ok(value) => value,
        Err(error) => return format!("error: invalid arguments JSON: {error}"),
    };

    // Gate order for a mutating call: turn-scoped Always-allow memory, then
    // the composer's ◇ permission picker (Auto-approve answers everything,
    // Read-only rejects), then the inline card.
    let class = tool.class();
    if !class.consent_flag(consent) {
        let gated = match router::permission_mode() {
            1 => true,
            2 => false,
            _ => match ask(class.label(), &gate_title(&args)) {
                PermissionDecision::Selected(id) if id == "allow_always" => {
                    class.remember(consent);
                    true
                }
                PermissionDecision::Selected(_) => true,
                _ => false,
            },
        };
        if !gated {
            let ctx = ToolContext { root, sink, jobs };
            return report(
                &ctx,
                "Rejected",
                rejected_title(&args),
                format!("user rejected this {}", class.label()),
            );
        }
    }

    let mut ctx = ToolContext { root, sink, jobs };
    tool.run(&args, &mut ctx, ask)
}

/// A human-readable one-line title for the permission card, derived from the
/// call arguments.
fn gate_title(args: &serde_json::Value) -> String {
    if let Some(path) = args["path"].as_str() {
        return path.to_string();
    }
    if let Some(command) = args["command"].as_str() {
        return format!("Run command: {command}");
    }
    if let Some(paths) = args["paths"].as_array() {
        return paths
            .iter()
            .filter_map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ");
    }
    "(unspecified target)".to_string()
}

fn rejected_title(args: &serde_json::Value) -> String {
    if let Some(command) = args["command"].as_str() {
        return format!("Bash: {command}");
    }
    if let Some(paths) = args["paths"].as_array() {
        return format!("Delete {}", gate_title(args));
    }
    gate_title(args)
}

/// Emit the transcript row and return the tool-result text.
pub fn report(ctx: &ToolContext, status: &str, primary: String, result: String) -> String {
    let _ = ctx.sink.send(eb_api::AgentEvent::Block {
        block: eb_api::model::Block::Tool {
            status: status.to_string(),
            primary,
            detail: None,
        },
    });
    result
}

/// Resolve a raw path against the workspace, refusing escapes: an absolute
/// path is used as-is only when it already lives under the workspace root,
/// so the model cannot reach outside the task workspace by writing `../..`.
/// The longest existing ancestor is canonicalized on both sides, so a
/// symlinked root (`/tmp` → `/private/tmp` on macOS) compares correctly, and
/// not-yet-existing files (a `write_file` target) resolve through their
/// existing parent.
pub fn resolve_in(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut existing = joined.clone();
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let base = existing.canonicalize().unwrap_or(existing);
    let resolved = missing
        .into_iter()
        .rev()
        .fold(base, |acc, part| acc.join(part));
    if resolved.starts_with(&root_canon) {
        Ok(resolved)
    } else {
        Err(format!(
            "path escapes the workspace: {raw} (workspace is {})",
            root.display()
        ))
    }
}

/// Keep the tail of `text` within `cap` bytes, cutting at a char boundary.
pub fn truncate_tail(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let start = text
        .char_indices()
        .map(|(index, _)| index)
        .find(|&index| index >= text.len() - cap)
        .unwrap_or(text.len());
    format!("[… head trimmed …]\n{}", &text[start..])
}

/// Command output kept per tool result, tail-trimmed. Shared by the tools
/// that return long text so one confused call cannot flood the context.
pub const TOOL_OUTPUT_CAP: usize = 8_000;

/// Escape a literal string for use as a regex pattern (the `regex` crate's
/// escape helper is not re-exported through grep-regex).
pub fn regex_escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for ch in literal.chars() {
        if matches!(
            ch,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
                | '#'
                | '-' | '&'
                | '~'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_keeps_paths_inside_the_workspace() {
        let root = std::env::temp_dir().join("eb-tools-resolve");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();

        assert!(resolve_in(&root, "a.txt").is_ok());
        assert!(resolve_in(&root, "./a.txt").is_ok());
        assert!(resolve_in(&root, &root.join("a.txt").display().to_string()).is_ok());
        let escaped = resolve_in(&root, "../../etc/passwd").unwrap_err();
        assert!(escaped.contains("escapes the workspace"), "{escaped}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn registry_is_stable_and_definitions_are_well_formed() {
        let names: Vec<_> = registry().iter().map(|t| t.name()).collect();
        assert_eq!(names.len(), 11);
        let expected = [
            "read_file",
            "list_dir",
            "write_file",
            "replace_in_file",
            "delete_files",
            "search_content",
            "search_file",
            "run_command",
            "command_output",
            "kill_command",
            "todo_write",
        ];
        assert_eq!(names, expected);
        for tool in registry() {
            assert!(!tool.description().is_empty(), "{} has no description", tool.name());
            assert!(tool.parameters().is_object());
        }
        let defs = tool_definitions();
        let first = &defs[0]["function"];
        assert_eq!(first["name"], "read_file");
        assert_eq!(first["parameters"]["required"], serde_json::json!(["path"]));
    }

    #[test]
    fn unknown_tool_reports_instead_of_panicking() {
        let (sink, _stream) = eb_api::AgentSink::channel();
        let mut consent = ToolConsent::default();
        let mut jobs = JobTable::default();
        let mut ask = |_class: &str, _title: &str| PermissionDecision::Cancelled;
        let root = std::env::temp_dir();
        let call = ToolCall {
            id: "t".into(),
            name: "nope".into(),
            arguments: "{}".into(),
        };
        let result = execute_tool(&call, &root, &mut consent, &mut ask, &sink, &mut jobs);
        assert!(result.contains("unknown tool"), "{result}");
    }

    /// Drive a tool by name through the real dispatcher (auto-approve), in
    /// a fresh temporary workspace.
    fn drive(root: &std::path::Path, name: &str, args: &str) -> String {
        let (sink, _stream) = eb_api::AgentSink::channel();
        let mut consent = ToolConsent::default();
        let mut jobs = JobTable::default();
        let mut ask =
            |_class: &str, _title: &str| PermissionDecision::Selected("allow_once".into());
        let call = ToolCall { id: "t".into(), name: name.into(), arguments: args.into() };
        crate::tools::execute_tool(&call, root, &mut consent, &mut ask, &sink, &mut jobs)
    }

    #[test]
    fn replace_in_file_requires_a_unique_match() {
        let root = std::env::temp_dir().join(format!("eb-tools-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("f.txt"), "alpha\nbeta\nalpha").unwrap();

        // Two matches must be refused.
        let two = drive(&root, "replace_in_file", r#"{"path":"f.txt","old_string":"alpha","new_string":"x"}"#);
        assert!(two.contains("matches 2 places"), "{two}");
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "alpha\nbeta\nalpha");

        // A unique, context-bearing match succeeds.
        let ok = drive(&root, "replace_in_file", r#"{"path":"f.txt","old_string":"beta\nalpha","new_string":"gamma"}"#);
        assert!(ok.contains("replaced"), "{ok}");
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "alpha\ngamma");

        // Zero matches name the problem.
        let zero = drive(&root, "replace_in_file", r#"{"path":"f.txt","old_string":"nope","new_string":"y"}"#);
        assert!(zero.contains("not found"), "{zero}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_content_finds_matches_and_skips_binary() {
        let root = std::env::temp_dir().join(format!("eb-tools-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("text.txt"), "hello world\nother line\nHELLO again\n").unwrap();
        std::fs::write(root.join("sub/deep.md"), "say hello deep\n").unwrap();
        // A NUL byte marks this as binary; the searcher must skip it.
        std::fs::write(root.join("blob.bin"), b"hello \x00\xff\xfe").unwrap();

        let hits = drive(&root, "search_content", r#"{"pattern":"hello","ignore_case":true}"#);
        assert!(hits.contains("text.txt:1: hello world"), "{hits}");
        assert!(hits.contains("deep.md"), "{hits}");
        assert!(!hits.contains("blob.bin"), "binary file leaked into results: {hits}");

        // Literal mode must not treat the pattern as a regex.
        let literal = drive(&root, "search_content", r#"{"pattern":"hello.","regex":false}"#);
        assert!(!literal.contains("HELLO again"), "{literal}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_file_globs_relative_paths() {
        let root = std::env::temp_dir().join(format!("eb-tools-glob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("src/a.rs"), "").unwrap();
        std::fs::write(root.join("src/deep/b.rs"), "").unwrap();
        std::fs::write(root.join("notes.md"), "").unwrap();

        let found = drive(&root, "search_file", r#"{"pattern":"**/*.rs"}"#);
        assert!(found.contains("src/a.rs"), "{found}");
        assert!(found.contains("src/deep/b.rs"), "{found}");
        assert!(!found.contains("notes.md"), "{found}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_files_refuses_the_workspace_root_and_escaped_paths() {
        let root = std::env::temp_dir().join(format!("eb-tools-delete-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let result = drive(&root, "delete_files", r#"{"paths":[".", "outside.txt"]}"#);
        assert!(result.contains("refusing to delete the workspace root"), "{result}");
    }

    #[test]
    fn todo_write_emits_a_plan_and_reports_progress() {
        let (sink, stream) = eb_api::AgentSink::channel();
        let mut consent = ToolConsent::default();
        let mut jobs = JobTable::default();
        let mut ask = |_c: &str, _t: &str| PermissionDecision::Selected("allow_once".into());
        let root = std::env::temp_dir();
        let call = ToolCall {
            id: "t".into(),
            name: "todo_write".into(),
            arguments: r#"{"todos":[{"content":"step one","status":"completed"},{"content":"step two","status":"in_progress"}]}"#.into(),
        };
        let result =
            crate::tools::execute_tool(&call, &root, &mut consent, &mut ask, &sink, &mut jobs);
        assert!(result.contains("1/2 completed"), "{result}");
        // The transcript received a Plan card with the statuses verbatim.
        let mut plan = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while plan.is_none() && std::time::Instant::now() < deadline {
            match stream.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(eb_api::AgentEvent::Block {
                    block: eb_api::model::Block::Plan { entries },
                }) => plan = Some(entries),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let plan = plan.expect("no Plan block emitted");
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].status, "completed");
        assert_eq!(plan[1].status, "in_progress");
    }

    #[test]
    fn background_commands_start_poll_and_die() {
        let (sink, _stream) = eb_api::AgentSink::channel();
        let mut consent = ToolConsent::default();
        let mut jobs = JobTable::default();
        let mut ask = |_c: &str, _t: &str| PermissionDecision::Selected("allow_once".into());
        let root = std::env::temp_dir().join(format!("eb-tools-bg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let call = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: r#"{"command":"echo bg-ready; sleep 30","run_in_background":true}"#.into(),
        };
        let started = crate::tools::execute_tool(
            &call, &root, &mut consent, &mut ask, &sink, &mut jobs,
        );
        assert!(started.contains("background job 1"), "{started}");

        // Poll until the echo lands (the job itself keeps sleeping).
        let mut output = String::new();
        for _ in 0..40 {
            let poll = ToolCall {
                id: "t".into(),
                name: "command_output".into(),
                arguments: r#"{"id":1}"#.into(),
            };
            output = crate::tools::execute_tool(
                &poll, &root, &mut consent, &mut ask, &sink, &mut jobs,
            );
            if output.contains("bg-ready") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(output.contains("bg-ready"), "{output}");
        assert!(output.contains("still running"), "{output}");

        let kill = ToolCall {
            id: "t".into(),
            name: "kill_command".into(),
            arguments: r#"{"id":1}"#.into(),
        };
        let killed = crate::tools::execute_tool(
            &kill, &root, &mut consent, &mut ask, &sink, &mut jobs,
        );
        assert!(killed.contains("killed"), "{killed}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn regex_escape_quotes_metacharacters() {
        assert_eq!(regex_escape("a.b*c"), r"a\.b\*c");
        assert_eq!(regex_escape("plain"), "plain");
    }
}
