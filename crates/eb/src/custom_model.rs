//! Custom OpenAI-compatible model answer source.
//!
//! A deliberately small adapter: streamed `/chat/completions` requests
//! through `curl` (SSE in, `AgentEvent::Delta`/`Thought` out), so custom
//! endpoints work without adding a TLS/HTTP stack to the GTK host. Non-SSE
//! answers (servers that ignore `stream`, plain JSON errors) still parse.
//!
//! The source also speaks OpenAI function calling: a small built-in toolset
//! (`read_file`, `write_file`, `list_dir`, `run_command`) is offered on every
//! request, executed against the task workspace, and fed back to the model in
//! the same turn. Mutating tools go through the host's permission prompter.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eb_api::{
    AgentEvent, AgentSink, AnswerHandle, AnswerSource, DenyAll, PermissionDecision,
    PermissionOption, PermissionPrompter, PermissionRequest,
};
use eb_api::model::Block;

/// Tool rounds per turn, so a confused model cannot loop forever.
const MAX_TOOL_ROUNDS: usize = 8;
/// Command output kept per tool result, tail-trimmed.
const TOOL_OUTPUT_CAP: usize = 8_000;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CustomConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub struct CustomModel {
    config: CustomConfig,
}

impl CustomModel {
    pub fn new(config: CustomConfig) -> Self {
        Self { config }
    }

    fn endpoint(&self) -> String {
        // The base URL is expected to carry its own version segment
        // (`.../v1`, `.../v4`, ...): appending a hardcoded `/v1` turned
        // bigmodel.cn's `/api/paas/v4` into `/v4/v1/chat/completions`.
        let base = self.config.base_url.trim().trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }

    /// The built-in toolset, in OpenAI tool schema.
    fn tool_definitions() -> serde_json::Value {
        serde_json::json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a UTF-8 text file. Relative paths resolve against the task workspace.",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string", "description": "File path" } },
                        "required": ["path"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "write_file",
                    "description": "Create or overwrite a UTF-8 text file (parents are created). Requires permission.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path" },
                            "content": { "type": "string", "description": "Full file content" }
                        },
                        "required": ["path", "content"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "list_dir",
                    "description": "List a directory's entries.",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string", "description": "Directory path, '.' for the workspace root" } },
                        "required": ["path"]
                    }
                }
            },
            {
                "type": "function",
                "function": {
                    "name": "run_command",
                    "description": "Run a shell command in the task workspace and capture its output. Requires permission.",
                    "parameters": {
                        "type": "object",
                        "properties": { "command": { "type": "string", "description": "Shell command" } },
                        "required": ["command"]
                    }
                }
            }
        ])
    }
}

impl AnswerSource for CustomModel {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        self.start_in(prompt, sink, Arc::new(DenyAll), None)
    }

    fn start_in(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: Option<PathBuf>,
    ) -> Box<dyn AnswerHandle> {
        if let Err(error) = validate(&self.config) {
            let _ = sink.send(AgentEvent::Failed { message: error });
            return Box::new(CustomHandle {
                cancelled: Arc::new(AtomicBool::new(false)),
                child: Arc::new(Mutex::new(None)),
            });
        }
        let _ = sink.send(AgentEvent::Started { name: self.config.name.clone() });
        // The host composes the prior visible conversation into `prompt`
        // (stateless channel), so the first message already carries context.
        let mut messages =
            vec![serde_json::json!({ "role": "user", "content": prompt })];
        let endpoint = self.endpoint();
        let key = self.config.api_key.clone();
        let base_url = self.config.base_url.clone();
        let model = self.config.model.clone();
        let root = workspace.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let child_for_handle = child.clone();
        let cancelled_for_worker = cancelled.clone();
        std::thread::Builder::new()
            .name("evobuddy-custom-model".into())
            .spawn(move || {
                for _round in 0..MAX_TOOL_ROUNDS {
                    let body = serde_json::json!({
                        "model": model,
                        "messages": messages,
                        "stream": true,
                        "tools": CustomModel::tool_definitions(),
                    });

                    let mut command = Command::new("curl");
                    command
                        .args([
                            "-sS",
                            "-N",
                            "--max-time",
                            "300",
                            "-X",
                            "POST",
                            // The JSON body arrives on stdin; without `-d @-`
                            // the request goes out with an empty body.
                            "-d",
                            "@-",
                            &endpoint,
                            "-H",
                            "Content-Type: application/json",
                            "-H",
                            "Accept: application/json",
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    if !key.trim().is_empty() {
                        command.arg("-H").arg(format!("Authorization: Bearer {key}"));
                    }
                    let Ok(mut process) = command.spawn() else {
                        let _ = sink.send(AgentEvent::Failed {
                            message: "cannot start curl".into(),
                        });
                        return;
                    };
                    // Take the stdin pipe so it drops right after the write:
                    // curl reads the body to EOF and must see the pipe close.
                    if let Some(mut stdin) = process.stdin.take() {
                        let _ = stdin.write_all(body.to_string().as_bytes());
                    }
                    if let Ok(mut slot) = child.lock() {
                        *slot = Some(process);
                    }

                    // Stream the SSE lines: content deltas land as Delta,
                    // reasoning chunks as Thought, and tool-call fragments
                    // accumulate for execution once the stream ends.
                    let stdout = child
                        .lock()
                        .ok()
                        .and_then(|mut slot| slot.as_mut().and_then(|p| p.stdout.take()));
                    let mut streamed = String::new();
                    let mut raw = String::new();
                    let mut stream_error = String::new();
                    let mut calls: BTreeMap<usize, ToolCall> = BTreeMap::new();
                    if let Some(stdout) = stdout {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines().map_while(Result::ok) {
                            if cancelled_for_worker.load(Ordering::SeqCst) {
                                break;
                            }
                            raw.push_str(&line);
                            raw.push('\n');
                            let payload = match line.strip_prefix("data:") {
                                Some(payload) => payload.trim(),
                                None => continue,
                            };
                            if payload == "[DONE]" {
                                break;
                            }
                            let Ok(value) =
                                serde_json::from_str::<serde_json::Value>(payload)
                            else {
                                continue;
                            };
                            if let Some(message) = value["error"]["message"].as_str() {
                                stream_error = message.to_string();
                                break;
                            }
                            let choice = &value["choices"][0];
                            let delta = &choice["delta"];
                            if let Some(piece) = delta["content"].as_str() {
                                if !piece.is_empty() {
                                    streamed.push_str(piece);
                                    let _ = sink.send(AgentEvent::Delta {
                                        text: piece.to_string(),
                                    });
                                }
                            }
                            for (key, event) in
                                [("reasoning_content", "thought"), ("reasoning", "thought")]
                            {
                                if let Some(piece) = delta[key].as_str() {
                                    if !piece.is_empty() {
                                        let _ = sink.send(AgentEvent::Thought {
                                            text: piece.to_string(),
                                        });
                                    }
                                }
                            }
                            if let Some(fragments) = delta["tool_calls"].as_array() {
                                for fragment in fragments {
                                    let index = fragment["index"].as_u64().unwrap_or(0) as usize;
                                    let entry = calls.entry(index).or_default();
                                    if let Some(id) = fragment["id"].as_str() {
                                        entry.id = id.to_string();
                                    }
                                    if let Some(name) =
                                        fragment["function"]["name"].as_str()
                                    {
                                        entry.name = name.to_string();
                                    }
                                    if let Some(arguments) =
                                        fragment["function"]["arguments"].as_str()
                                    {
                                        entry.arguments.push_str(arguments);
                                    }
                                }
                            }
                        }
                    }

                    // Drain stderr and wait for the exit so the handle's kill
                    // stays meaningful to the very end.
                    let errors = {
                        let mut errors = String::new();
                        if let Ok(mut slot) = child.lock() {
                            if let Some(process) = slot.as_mut() {
                                if let Some(mut stderr) = process.stderr.take() {
                                    let _ = stderr.read_to_string(&mut errors);
                                }
                                let _ = process.wait();
                            }
                        }
                        errors
                    };
                    if cancelled_for_worker.load(Ordering::SeqCst) {
                        let _ = sink.send(AgentEvent::Failed { message: "Cancelled".into() });
                        return;
                    }

                    let calls: Vec<ToolCall> = calls.into_values().collect();
                    if calls.is_empty() {
                        // No tools requested: the streamed content (or the
                        // plain-JSON fallback) is the answer.
                        if !stream_error.is_empty() {
                            let _ = sink.send(AgentEvent::Failed {
                                message: hint_for(&stream_error, &base_url),
                            });
                            return;
                        }
                        if !streamed.is_empty() {
                            let _ = sink.send(AgentEvent::Finished);
                            return;
                        }
                        // A server that ignored `stream` answered with one
                        // plain JSON body: parse it as the answer.
                        let failure = match parse_response(&raw) {
                            Ok(text) => {
                                let _ = sink.send(AgentEvent::Block {
                                    block: Block::Text(text),
                                });
                                let _ = sink.send(AgentEvent::Finished);
                                return;
                            }
                            Err(error) => error,
                        };
                        let detail = errors.lines().last().unwrap_or("").trim().to_string();
                        let message = if detail.is_empty() {
                            failure
                        } else {
                            format!("{failure}: {detail}")
                        };
                        let _ = sink.send(AgentEvent::Failed {
                            message: hint_for(&message, &base_url),
                        });
                        return;
                    }

                    // Tool round: append the assistant's tool_calls, execute
                    // each, and feed the results back for the next round.
                    let calls_json: Vec<serde_json::Value> = calls
                        .iter()
                        .map(|call| {
                            serde_json::json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments,
                                }
                            })
                        })
                        .collect();
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "tool_calls": calls_json,
                    }));
                    for call in &calls {
                        let outcome = execute_tool(call, &root, &permissions, &sink);
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": outcome,
                        }));
                    }
                }
                let _ = sink.send(AgentEvent::Failed {
                    message: format!(
                        "tool loop exceeded {MAX_TOOL_ROUNDS} rounds without a final answer"
                    ),
                });
            })
            .ok();
        Box::new(CustomHandle {
            cancelled,
            child: child_for_handle,
        })
    }
}

/// One accumulated tool call from the stream, in arrival (index) order.
#[derive(Default)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Execute one built-in tool. Mutating tools go through the permission
/// prompter; a rejection is reported back to the model as the tool result.
fn execute_tool(
    call: &ToolCall,
    root: &std::path::Path,
    permissions: &Arc<dyn PermissionPrompter>,
    sink: &AgentSink,
) -> String {
    let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
        Ok(value) => value,
        Err(error) => return format!("error: invalid arguments JSON: {error}"),
    };
    let resolve = |raw: &str| {
        let path = std::path::Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let report = |status: &str, primary: String, result: String, sink: &AgentSink| {
        let _ = sink.send(AgentEvent::Block {
            block: Block::Tool {
                status: status.to_string(),
                primary,
                detail: None,
            },
        });
        result
    };

    match call.name.as_str() {
        "read_file" => {
            let raw = args["path"].as_str().unwrap_or_default();
            let path = resolve(raw);
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let trimmed = truncate_tail(&content, TOOL_OUTPUT_CAP);
                    report(
                        "Ran",
                        format!("Read {raw}"),
                        if content.len() > trimmed.len() {
                            format!("[… head trimmed …]\n{trimmed}")
                        } else {
                            trimmed
                        },
                        sink,
                    )
                }
                Err(error) => report(
                    "Failed",
                    format!("Read {raw}"),
                    format!("read failed: {error}"),
                    sink,
                ),
            }
        }
        "write_file" => {
            let raw = args["path"].as_str().unwrap_or_default().to_string();
            let content = args["content"].as_str().unwrap_or_default();
            if !allow(permissions, &format!("Write file: {raw}")) {
                return report(
                    "Rejected",
                    format!("Write {raw}"),
                    "user rejected this write".into(),
                    sink,
                );
            }
            let path = resolve(&raw);
            let write = std::fs::create_dir_all(
                path.parent().unwrap_or(std::path::Path::new(".")),
            )
            .and_then(|_| std::fs::write(&path, content));
            match write {
                Ok(()) => report(
                    "Ran",
                    format!("Write {raw}"),
                    format!("wrote {} bytes to {raw}", content.len()),
                    sink,
                ),
                Err(error) => report(
                    "Failed",
                    format!("Write {raw}"),
                    format!("write failed: {error}"),
                    sink,
                ),
            }
        }
        "list_dir" => {
            let raw = args["path"].as_str().unwrap_or(".").to_string();
            let path = resolve(&raw);
            let mut entries: Vec<String> = Vec::new();
            match std::fs::read_dir(&path) {
                Ok(read) => {
                    for entry in read.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let mark =
                            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                "/"
                            } else {
                                ""
                            };
                        entries.push(format!("{name}{mark}"));
                    }
                    entries.sort();
                    let listing = entries.join("\n");
                    report(
                        "Ran",
                        format!("List {raw}"),
                        truncate_tail(&listing, TOOL_OUTPUT_CAP),
                        sink,
                    )
                }
                Err(error) => report(
                    "Failed",
                    format!("List {raw}"),
                    format!("list failed: {error}"),
                    sink,
                ),
            }
        }
        "run_command" => {
            let command = args["command"].as_str().unwrap_or_default().to_string();
            if !allow(permissions, &format!("Run command: {command}")) {
                return report(
                    "Rejected",
                    format!("Bash: {command}"),
                    "user rejected this command".into(),
                    sink,
                );
            }
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(root)
                .stdin(Stdio::null())
                .output();
            match output {
                Ok(output) => {
                    let mut combined = String::new();
                    combined.push_str(&String::from_utf8_lossy(&output.stdout));
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.trim().is_empty() {
                        combined.push_str(&stderr);
                    }
                    let code = output.status.code().unwrap_or(-1);
                    let body = truncate_tail(combined.trim(), TOOL_OUTPUT_CAP);
                    report(
                        if code == 0 { "Ran" } else { "Failed" },
                        format!("Bash: {command}"),
                        format!("exit {code}\n{body}"),
                        sink,
                    )
                }
                Err(error) => report(
                    "Failed",
                    format!("Bash: {command}"),
                    format!("spawn failed: {error}"),
                    sink,
                ),
            }
        }
        other => format!("error: unknown tool {other}"),
    }
}

/// Ask the host's permission prompter for a mutating tool call.
fn allow(permissions: &Arc<dyn PermissionPrompter>, title: &str) -> bool {
    let request = PermissionRequest {
        title: title.to_string(),
        options: vec![
            PermissionOption {
                id: "allow_once".into(),
                label: "Allow".into(),
                kind: "allow_once".into(),
            },
            PermissionOption {
                id: "reject_once".into(),
                label: "Reject".into(),
                kind: "reject_once".into(),
            },
        ],
    };
    matches!(
        permissions.ask(request),
        PermissionDecision::Selected(id) if id == "allow_once"
    )
}

/// Keep the tail of `text` within `cap` bytes, cutting at a char boundary.
fn truncate_tail(text: &str, cap: usize) -> String {
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

struct CustomHandle {
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl AnswerHandle for CustomHandle {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.child.lock() {
            if let Some(process) = slot.as_mut() {
                let _ = process.kill();
            }
        }
    }
}

fn validate(config: &CustomConfig) -> Result<(), String> {
    if config.base_url.trim().is_empty() {
        return Err("custom model Base URL is required".into());
    }
    if !config.base_url.starts_with("http://") && !config.base_url.starts_with("https://") {
        return Err("custom model Base URL must be http(s)".into());
    }
    if config.model.trim().is_empty() {
        return Err("custom model id is required".into());
    }
    Ok(())
}

fn parse_response(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("invalid JSON: {error}"))?;
    if let Some(message) = value["error"]["message"].as_str() {
        return Err(message.to_string());
    }
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "response has no choices[0].message.content".into())
}

///bigmodel.cn 的编程套餐配额只在其 coding 端点生效;普通 paas 端点对套餐模型
/// 返回"余额不足/无权限"。识别这组错误并直接给出改 Base URL 的指引。
fn hint_for(message: &str, base_url: &str) -> String {
    let base = base_url.to_lowercase();
    let plan_related = message.contains("余额不足")
        || message.contains("无可用资源包")
        || message.contains("No permission to access model");
    if base.contains("bigmodel.cn") && !base.contains("/coding/") && plan_related {
        format!(
            "{message}\n提示:智谱编程套餐的配额只在 coding 端点生效,请把 Base URL 改为 https://open.bigmodel.cn/api/coding/paas/v4"
        )
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn endpoint_accepts_base_v1_and_full_paths() {
        let mut config = CustomConfig { base_url: "https://example.test/v1".into(), model: "m".into(), ..Default::default() };
        let model = CustomModel::new(config.clone());
        assert_eq!(model.endpoint(), "https://example.test/v1/chat/completions");
        // A version segment other than v1 must survive: the old hardcoded /v1
        // produced /v4/v1/chat/completions on bigmodel.cn.
        config.base_url = "https://open.bigmodel.cn/api/paas/v4".into();
        assert_eq!(
            CustomModel::new(config.clone()).endpoint(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        config.base_url = "https://example.test/v1/chat/completions".into();
        assert_eq!(CustomModel::new(config).endpoint(), "https://example.test/v1/chat/completions");
    }

    struct AllowWrites;
    impl PermissionPrompter for AllowWrites {
        fn ask(
            &self,
            _request: PermissionRequest,
        ) -> PermissionDecision {
            PermissionDecision::Selected("allow_once".into())
        }
    }

    #[test]
    fn bigmodel_plan_errors_get_the_coding_endpoint_hint() {
        let plain = hint_for("No permission to access model: GLM-5.3", "https://open.bigmodel.cn/api/paas/v4");
        assert!(plain.contains("/api/coding/paas/v4"), "hint missing: {plain}");

        // A coding endpoint, or another provider, must stay untouched.
        let already = hint_for("余额不足", "https://open.bigmodel.cn/api/coding/paas/v4");
        assert_eq!(already, "余额不足");
        let other = hint_for("余额不足", "https://api.example.com/v1");
        assert_eq!(other, "余额不足");
    }

    #[test]
    fn response_extracts_content() {
        let body = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
        assert_eq!(parse_response(body).unwrap(), "hello");
    }

    /// The full turn against a real local HTTP server: this is what caught
    /// the missing `-d @-` — curl was POSTing an empty body while the request
    /// JSON sat unread on its stdin.
    #[test]
    fn a_turn_posts_the_body_and_streams_the_answer() {
        if !python_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        use std::io::BufRead as _;
        let mut server = std::process::Command::new("python3")
            .args(["tests/fixtures/openai_mock.py"])
            .env("MOCK_MODE", "plain")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start mock server");
        let address = {
            let stdout = server.stdout.take().expect("mock server stdout");
            let mut lines = BufReader::new(stdout).lines();
            lines.next().expect("mock server prints its address").unwrap()
        };

        let model = CustomModel::new(CustomConfig {
            name: "mock".into(),
            base_url: format!("http://{address}/v1"),
            api_key: "test-key".into(),
            model: "mock-model".into(),
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        let handle = model.start("what is 2 + 2", sink);
        let mut text = String::new();
        let mut deltas = 0usize;
        let mut thoughts = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let finished = loop {
            if std::time::Instant::now() > deadline {
                break false;
            }
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(eb_api::AgentEvent::Delta { text: value }) => {
                    deltas += 1;
                    text.push_str(&value);
                }
                Ok(eb_api::AgentEvent::Thought { text: value }) => {
                    thoughts += 1;
                    assert_eq!(value, "thinking hard");
                }
                Ok(eb_api::AgentEvent::Block { block: Block::Text(value) }) => text.push_str(&value),
                Ok(eb_api::AgentEvent::Failed { message }) => {
                    panic!("turn failed: {message}");
                }
                Ok(eb_api::AgentEvent::Finished) => break true,
                Ok(_) => {}
                Err(_) => break false,
            }
        };
        handle.cancel();
        let _ = server.kill();
        assert!(finished, "the turn never finished");
        assert_eq!(text, "mock says: what is 2 + 2");
        assert!(deltas >= 2, "the answer should arrive as streamed deltas, got {deltas}");
        assert_eq!(thoughts, 1, "reasoning_content must stream as a thought");
    }

    /// A tool round against the mock: the model asks for write_file, the
    /// client executes it in the workspace, and the tool result flows back so
    /// the model can answer.
    #[test]
    fn tool_calls_execute_in_the_workspace_and_feed_back() {
        if !python_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        use std::io::BufRead as _;
        let workspace = std::env::temp_dir().join(format!(
            "eb-custom-tools-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).unwrap();

        let mut server = std::process::Command::new("python3")
            .args(["tests/fixtures/openai_mock.py"])
            .env("MOCK_MODE", "tools")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start mock server");
        let address = {
            let stdout = server.stdout.take().expect("mock server stdout");
            let mut lines = BufReader::new(stdout).lines();
            lines.next().expect("mock server prints its address").unwrap()
        };

        let model = CustomModel::new(CustomConfig {
            name: "mock".into(),
            base_url: format!("http://{address}/v1"),
            api_key: "test-key".into(),
            model: "mock-model".into(),
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        let handle = model.start_in(
            "write it",
            sink,
            Arc::new(AllowWrites),
            Some(workspace.clone()),
        );
        let mut text = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let finished = loop {
            if std::time::Instant::now() > deadline {
                break false;
            }
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(eb_api::AgentEvent::Delta { text: value }) => text.push_str(&value),
                Ok(eb_api::AgentEvent::Block { block: Block::Tool { status, primary, .. } }) => {
                    assert!(primary.starts_with("Write "));
                    assert_eq!(status, "Ran");
                }
                Ok(eb_api::AgentEvent::Failed { message }) => {
                    panic!("turn failed: {message}");
                }
                Ok(eb_api::AgentEvent::Finished) => break true,
                Ok(_) => {}
                Err(_) => break false,
            }
        };
        handle.cancel();
        let _ = server.kill();
        assert!(finished, "the tool round never finished");
        assert_eq!(text, "tool result seen");
        let written =
            std::fs::read_to_string(workspace.join("tool-test/out.txt")).unwrap();
        assert_eq!(written, "hello from the model");
        std::fs::remove_dir_all(&workspace).unwrap();
    }

    #[test]
    fn denials_are_reported_as_tool_results() {
        let (sink, _stream) = eb_api::AgentSink::channel();
        let permissions: Arc<dyn PermissionPrompter> = Arc::new(DenyAll);
        let call = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: r#"{"command": "echo hi"}"#.into(),
        };
        let result = execute_tool(
            &call,
            std::path::Path::new("/tmp"),
            &permissions,
            &sink,
        );
        assert_eq!(result, "user rejected this command");
    }

    #[test]
    fn read_file_lists_and_errors() {
        let (sink, _stream) = eb_api::AgentSink::channel();
        let permissions: Arc<dyn PermissionPrompter> = Arc::new(DenyAll);
        let dir = std::env::temp_dir().join(format!("eb-custom-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "abc").unwrap();

        let read = ToolCall {
            id: "t".into(),
            name: "read_file".into(),
            arguments: json_args(r#"{"path": "a.txt"}"#),
        };
        assert_eq!(
            execute_tool(&read, &dir, &permissions, &sink),
            "abc"
        );
        let list = ToolCall {
            id: "t".into(),
            name: "list_dir".into(),
            arguments: json_args(r#"{"path": "."}"#),
        };
        let listing = execute_tool(&list, &dir, &permissions, &sink);
        assert!(listing.contains("a.txt") && listing.contains("sub/"));
        let missing = ToolCall {
            id: "t".into(),
            name: "read_file".into(),
            arguments: json_args(r#"{"path": "nope.txt"}"#),
        };
        assert!(execute_tool(&missing, &dir, &permissions, &sink).starts_with("read failed:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn json_args(raw: &str) -> String {
        raw.to_string()
    }

    fn python_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
