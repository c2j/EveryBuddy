//! ACP (Agent Client Protocol) answer source.
//!
//! This is the real-agent seam the demo agent only stood in for. It launches an
//! external ACP agent (`dsh --profile acp`, `opencode acp`, …), speaks
//! newline-delimited JSON-RPC over stdio, creates one persistent session, and
//! maps `session/update` notifications onto EvoBuddy's [`AgentEvent`] stream.
//!
//! Configuration is environment-only:
//!
//! | env | meaning |
//! |---|---|
//! | `EVOBUDDY_ACP_COMMAND` | required; executable to launch |
//! | `EVOBUDDY_ACP_ARGS` | args, whitespace-separated or a JSON string array |
//! | `EVOBUDDY_ACP_CWD` | absolute session workspace (default: process cwd) |
//! | `EVOBUDDY_ACP_AUTO_APPROVE` | `1` auto-selects allow options; default rejects |
//! | `EVOBUDDY_ACP_NAME` | fallback display name if the agent reports none |
//! | `EVOBUDDY_ACP_DEBUG` | forward the agent's stderr |
//!
//! The plugin is selected by the host when `EVOBUDDY_ACP_COMMAND` is set;
//! otherwise the demo agent remains the answer source.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eb_api::model::{Block, DiffLine, PlanEntry};
use eb_api::{
    AgentEvent, AgentSink, AnswerHandle, AnswerSource, DenyAll, EbPlugin, PermissionDecision,
    PermissionOption, PermissionPrompter, PermissionRequest, Ui,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: u64 = 1;

#[derive(Clone, Debug)]
pub struct AcpConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub auto_approve: bool,
    pub name: String,
    pub debug: bool,
    pub env: Vec<(String, String)>,
    pub auth_method: Option<String>,
}

impl AcpConfig {
    pub fn from_env() -> Option<Self> {
        let program = std::env::var("EVOBUDDY_ACP_COMMAND").ok()?;
        let program = program.trim().to_string();
        if program.is_empty() {
            return None;
        }
        let args = std::env::var("EVOBUDDY_ACP_ARGS")
            .ok()
            .map(|raw| parse_args(&raw))
            .unwrap_or_default();
        let cwd = std::env::var("EVOBUDDY_ACP_CWD")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let cwd = if cwd.is_absolute() {
            cwd
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(cwd)
        };
        let auto_approve = std::env::var("EVOBUDDY_ACP_AUTO_APPROVE")
            .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        let name = std::env::var("EVOBUDDY_ACP_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "ACP agent".to_string());
        let debug = std::env::var_os("EVOBUDDY_ACP_DEBUG").is_some();
        Some(AcpConfig {
            program,
            args,
            cwd,
            auto_approve,
            name,
            debug,
            env: Vec::new(),
            auth_method: std::env::var("EVOBUDDY_ACP_AUTH_METHOD")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        })
    }
}

fn parse_args(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        if let Ok(values) = serde_json::from_str::<Vec<String>>(trimmed) {
            return values;
        }
    }
    trimmed
        .split_whitespace()
        .map(ToString::to_string)
        .collect()
}

#[derive(Clone)]
pub struct AcpAgent {
    config: Arc<AcpConfig>,
    session: Arc<Mutex<Option<AcpSession>>>,
    cancel: Arc<Mutex<CancelState>>,
}

#[derive(Default)]
struct CancelState {
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    session_id: Option<String>,
}

impl AcpAgent {
    pub fn new(config: AcpConfig) -> Self {
        AcpAgent {
            config: Arc::new(config),
            session: Arc::new(Mutex::new(None)),
            cancel: Arc::new(Mutex::new(CancelState::default())),
        }
    }

    pub fn from_env() -> Option<Self> {
        AcpConfig::from_env().map(Self::new)
    }

    fn run(
        &self,
        prompt: String,
        sink: AgentSink,
        cancelled: Arc<AtomicBool>,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: std::path::PathBuf,
    ) {
        // The external agent may take seconds to boot (opencode/dsh load their
        // own runtime). Acknowledge immediately so the composer switches to its
        // stop state instead of looking like the click did nothing.
        if sink
            .send(AgentEvent::Started {
                name: self.config.name.clone(),
            })
            .is_err()
        {
            return;
        }
        let _ = sink.send(AgentEvent::Status {
            text: "Starting agent…".to_string(),
        });

        let mut slot = lock(&self.session);
        if slot.as_ref().is_some_and(|session| session.cwd != workspace) {
            *slot = None;
        }
        if slot.is_none() {
            if self.config.debug {
                eprintln!("acp: launching {}", self.config.program);
            }
            match AcpSession::launch(&self.config, permissions.clone(), &workspace) {
                Ok(session) => {
                    let mut cancel = lock(&self.cancel);
                    cancel.stdin = Some(session.stdin.clone());
                    cancel.session_id = Some(session.session_id.clone());
                    *slot = Some(session);
                }
                Err(error) => {
                    let _ = sink.send(AgentEvent::Failed { message: error });
                    return;
                }
            }
        }
        let Some(session) = slot.as_mut() else {
            return;
        };
        session.permissions = permissions;
        if sink
            .send(AgentEvent::Started {
                name: session.agent_name.clone(),
            })
            .is_err()
        {
            return;
        }
        let _ = sink.send(AgentEvent::Status {
            text: "Thinking".to_string(),
        });
        if cancelled.load(Ordering::SeqCst) {
            let _ = sink.send(AgentEvent::Failed {
                message: "Cancelled".to_string(),
            });
            return;
        }
        match session.prompt(&prompt, &sink, &cancelled) {
            Ok(()) => {
                let _ = sink.send(AgentEvent::Finished);
            }
            Err(PromptError::Cancelled) => {
                let _ = sink.send(AgentEvent::Failed {
                    message: "Cancelled".to_string(),
                });
            }
            Err(PromptError::Message(message)) => {
                let _ = sink.send(AgentEvent::Failed { message });
                // A protocol-level failure may leave the pipe half-consumed;
                // relaunch on the next prompt rather than reusing it.
                *slot = None;
            }
        }
    }
}

impl EbPlugin for AcpAgent {
    fn id(&self) -> &'static str {
        "acp-agent"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        let source: Arc<dyn AnswerSource> = Arc::new(self.clone());
        eb_api::provide_answer(ui.ctx, source).map(|_| ())
    }
}

impl AnswerSource for AcpAgent {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        self.start_with_permissions(prompt, sink, Arc::new(DenyAll))
    }

    fn start_with_permissions(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
    ) -> Box<dyn AnswerHandle> {
        if self.config.debug || std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "acp: start ({} chars, command={})",
                prompt.chars().count(),
                self.config.program
            );
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = AcpHandle {
            cancelled: cancelled.clone(),
            cancel: self.cancel.clone(),
        };
        let agent = self.clone();
        let prompt = prompt.to_string();
        let cwd = self.config.cwd.clone();
        std::thread::Builder::new()
            .name("evobuddy-acp".to_string())
            .spawn(move || agent.run(prompt, sink, cancelled, permissions, cwd))
            .expect("spawn ACP worker");
        Box::new(handle)
    }

    fn start_in(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: Option<std::path::PathBuf>,
    ) -> Box<dyn AnswerHandle> {
        let cwd = workspace.unwrap_or_else(|| self.config.cwd.clone());
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = AcpHandle {
            cancelled: cancelled.clone(),
            cancel: self.cancel.clone(),
        };
        let agent = self.clone();
        let prompt = prompt.to_string();
        std::thread::Builder::new()
            .name("evobuddy-acp".to_string())
            .spawn(move || agent.run(prompt, sink, cancelled, permissions, cwd))
            .expect("spawn ACP worker");
        Box::new(handle)
    }
}

struct AcpHandle {
    cancelled: Arc<AtomicBool>,
    cancel: Arc<Mutex<CancelState>>,
}

impl AnswerHandle for AcpHandle {
    fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = lock(&self.cancel);
        let (Some(stdin), Some(session_id)) = (&state.stdin, &state.session_id) else {
            return;
        };
        let message = json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": session_id }
        });
        let _ = write_json(stdin, &message);
    }
}

struct AcpSession {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    session_id: String,
    cwd: std::path::PathBuf,
    agent_name: String,
    auto_approve: bool,
    permissions: Arc<dyn PermissionPrompter>,
    debug: bool,
}

impl AcpSession {
    fn launch(
        config: &AcpConfig,
        permissions: Arc<dyn PermissionPrompter>,
        cwd: &std::path::Path,
    ) -> Result<Self, String> {
        let mut command = Command::new(&config.program);
        command
            .args(&config.args)
            .envs(config.env.iter().map(|(key, value)| (key, value)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if config.debug {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not launch '{}': {error}", config.program))?;
        let stdin = Arc::new(Mutex::new(
            child.stdin.take().ok_or("agent stdin was not piped")?,
        ));
        let stdout = BufReader::new(child.stdout.take().ok_or("agent stdout was not piped")?);
        if config.debug {
            if let Some(stderr) = child.stderr.take() {
                std::thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        eprintln!("acp: {line}");
                    }
                });
            }
        }

        let mut session = AcpSession {
            child,
            stdin,
            stdout,
            next_id: 1,
            session_id: String::new(),
            cwd: cwd.to_path_buf(),
            agent_name: config.name.clone(),
            auto_approve: config.auto_approve,
            permissions,
            debug: config.debug,
        };

        if config.debug {
            eprintln!("acp: initialize cwd={}", cwd.display());
        }
        let init = session.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {},
                "clientInfo": {
                    "name": "evobuddy",
                    "title": "EvoBuddy",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )?;
        let version = init
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if version != PROTOCOL_VERSION {
            return Err(format!(
                "ACP version {version} is not supported (need {PROTOCOL_VERSION})"
            ));
        }
        if let Some(method) = &config.auth_method {
            session.request("authenticate", json!({ "methodId": method }))?;
        }
        if let Some(name) = init
            .pointer("/agentInfo/title")
            .or_else(|| init.pointer("/agentInfo/name"))
            .and_then(Value::as_str)
        {
            session.agent_name = name.to_string();
        }

        let cwd = cwd.to_string_lossy();
        let new_session =
            session.request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))?;
        let session_id = new_session
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("session/new returned no sessionId")?
            .to_string();
        if session.debug {
            eprintln!("acp: session ready: {session_id}");
        }
        session.session_id = session_id;
        Ok(session)
    }

    fn prompt(
        &mut self,
        prompt: &str,
        sink: &AgentSink,
        cancelled: &AtomicBool,
    ) -> Result<(), PromptError> {
        let id = self.next_id;
        self.next_id += 1;
        if self.debug {
            eprintln!("acp: session/prompt {id}");
        }
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": {
                "sessionId": self.session_id,
                "prompt": [{ "type": "text", "text": prompt }]
            }
        }))
        .map_err(PromptError::Message)?;

        let mut tools: HashMap<String, ToolState> = HashMap::new();
        let mut pending_tools: Vec<Block> = Vec::new();
        let mut cancel_sent = false;
        loop {
            if cancelled.load(Ordering::SeqCst) && !cancel_sent {
                cancel_sent = true;
                let _ = self.write(&json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": self.session_id }
                }));
            }

            let message = self.read().map_err(PromptError::Message)?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if self.debug {
                    eprintln!("acp: prompt response {id}");
                }
                if let Some(error) = message.get("error") {
                    return Err(PromptError::Message(format!(
                        "session/prompt failed: {}",
                        error
                    )));
                }
                let stop = message
                    .pointer("/result/stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn");
                flush_tool_group(sink, &mut pending_tools).map_err(PromptError::Message)?;
                if stop == "cancelled" {
                    return Err(PromptError::Cancelled);
                }
                return Ok(());
            }

            let Some(method) = message.get("method").and_then(Value::as_str) else {
                continue;
            };
            if message.get("id").is_some() {
                self.respond_request(&message)
                    .map_err(PromptError::Message)?;
                continue;
            }
            if method == "session/update" {
                map_update(&message["params"]["update"], sink, &mut tools, &mut pending_tools)
                    .map_err(PromptError::Message)?;
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;
        loop {
            let message = self.read()?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("{method} failed: {error}"));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            if message.get("method").is_some() && message.get("id").is_some() {
                self.respond_request(&message)?;
            }
        }
    }

    fn respond_request(&mut self, message: &Value) -> Result<(), String> {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "session/request_permission" {
            let raw_options = message
                .pointer("/params/options")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let options: Vec<PermissionOption> = raw_options
                .iter()
                .filter_map(|option| {
                    Some(PermissionOption {
                        id: option.get("optionId")?.as_str()?.to_string(),
                        label: option
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("Allow")
                            .to_string(),
                        kind: option
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("other")
                            .to_string(),
                    })
                })
                .collect();
            let title = message
                .pointer("/params/toolCall/title")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Tool permission")
                .to_string();
            if self.debug {
                eprintln!(
                    "acp: permission request {:?} ({} options, auto={})",
                    title,
                    options.len(),
                    self.auto_approve
                );
            }
            let selected = if self.auto_approve {
                pick_permission(&raw_options, true)
            } else {
                match self.permissions.ask(PermissionRequest { title, options }) {
                    PermissionDecision::Selected(id) => Some(id),
                    PermissionDecision::Cancelled => None,
                }
            };
            let selected = selected.filter(|id| {
                raw_options
                    .iter()
                    .any(|option| option.get("optionId").and_then(Value::as_str) == Some(id))
            });
            let outcome = match selected {
                Some(option_id) => json!({
                    "outcome": { "outcome": "selected", "optionId": option_id }
                }),
                None => json!({ "outcome": { "outcome": "cancelled" } }),
            };
            return self.write(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": outcome
            }));
        }
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("unsupported client method: {method}") }
        }))
    }

    fn write(&mut self, value: &Value) -> Result<(), String> {
        write_json(&self.stdin, value)
    }

    fn read(&mut self) -> Result<Value, String> {
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("read from ACP agent: {error}"))?;
            if read == 0 {
                return Err("ACP agent closed stdout".to_string());
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            return serde_json::from_str(line)
                .map_err(|error| format!("invalid ACP JSON: {error}: {line}"));
        }
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_json(stdin: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<(), String> {
    let mut data = serde_json::to_string(value).map_err(|error| error.to_string())?;
    data.push('\n');
    let mut stdin = lock(stdin);
    stdin
        .write_all(data.as_bytes())
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("write to ACP agent: {error}"))
}

fn pick_permission(options: &[Value], auto_approve: bool) -> Option<String> {
    let preferred: &[&str] = if auto_approve {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    for kind in preferred {
        if let Some(option) = options
            .iter()
            .find(|option| option.get("kind").and_then(Value::as_str) == Some(kind))
        {
            if let Some(id) = option.get("optionId").and_then(Value::as_str) {
                return Some(id.to_string());
            }
        }
    }
    None
}

#[derive(Default, Clone)]
struct ToolState {
    title: String,
    kind: String,
    status: String,
    command: Option<String>,
    output: String,
    diffs: Vec<(String, String, String)>,
}

fn map_update(
    update: &Value,
    sink: &AgentSink,
    tools: &mut HashMap<String, ToolState>,
    pending_tools: &mut Vec<Block>,
) -> Result<(), String> {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            flush_tool_group(sink, pending_tools)?;
            if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                send(
                    sink,
                    AgentEvent::Delta {
                        text: text.to_string(),
                    },
                )?;
            }
        }
        "usage_update" => {
            let used = update.get("used").and_then(Value::as_u64).unwrap_or(0);
            let total = update.get("size").and_then(Value::as_u64).unwrap_or(0);
            if total > 0 {
                send(
                    sink,
                    AgentEvent::Usage {
                        used: used.min(u32::MAX as u64) as u32,
                        total: total.min(u32::MAX as u64) as u32,
                    },
                )?;
            }
        }
        "plan" => {
            let entries: Vec<PlanEntry> = update
                .get("entries")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| {
                            Some(PlanEntry {
                                content: entry.get("content")?.as_str()?.to_string(),
                                status: entry
                                    .get("status")
                                    .and_then(Value::as_str)
                                    .unwrap_or("pending")
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !entries.is_empty() {
                flush_tool_group(sink, pending_tools)?;
                send(sink, AgentEvent::Block { block: Block::Plan { entries } })?;
            }
        }
        "tool_call" | "tool_call_update" => {
            let id = update
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                return Ok(());
            }
            let state = tools.entry(id).or_default();
            merge_tool(state, update);
            if kind == "tool_call" {
                if !state.title.is_empty() {
                    send(
                        sink,
                        AgentEvent::Status {
                            text: state.title.clone(),
                        },
                    )?;
                }
            } else if matches!(state.status.as_str(), "completed" | "failed") {
                emit_tool(state, pending_tools);
            }
        }
        _ => {}
    }
    Ok(())
}

fn merge_tool(state: &mut ToolState, update: &Value) {
    if let Some(title) = update.get("title").and_then(Value::as_str) {
        if !title.is_empty() {
            state.title = title.to_string();
        }
    }
    if let Some(kind) = update.get("kind").and_then(Value::as_str) {
        state.kind = kind.to_string();
    }
    if let Some(status) = update.get("status").and_then(Value::as_str) {
        state.status = status.to_string();
    }
    if let Some(input) = update.get("rawInput") {
        state.command = input
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| input.get("cmd").and_then(Value::as_str))
            .map(ToString::to_string)
            .or_else(|| input.as_str().map(ToString::to_string))
            .or(state.command.take());
    }
    if let Some(content) = update.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("content") => {
                    if let Some(text) = item.pointer("/content/text").and_then(Value::as_str) {
                        if !state.output.is_empty() {
                            state.output.push('\n');
                        }
                        state.output.push_str(text);
                    }
                }
                Some("diff") => {
                    let path = item
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("file")
                        .to_string();
                    let old = item
                        .get("oldText")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let new = item
                        .get("newText")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    state.diffs.push((path, old, new));
                }
                _ => {}
            }
        }
    }
}

fn emit_tool(state: &ToolState, pending: &mut Vec<Block>) {
    let status = if state.status == "completed" {
        "Ran"
    } else {
        "Failed"
    };
    if !state.diffs.is_empty() {
        for (path, old, new) in &state.diffs {
            let (lines, added, removed) = diff_lines(old, new);
            pending.push(Block::Diff {
                path: path.clone(),
                added,
                removed,
                lines,
            });
        }
        return;
    }
    if state.kind == "execute" {
        if let Some(command) = &state.command {
            pending.push(Block::Exec {
                command: command.clone(),
                output: state.output.clone(),
                exit_code: if state.status == "completed" {
                    Some(0)
                } else {
                    Some(1)
                },
            });
            return;
        }
    }
    let detail = if state.output.is_empty() {
        None
    } else {
        Some(first_line(&state.output, 80))
    };
    pending.push(Block::Tool {
        status: status.to_string(),
        primary: if state.title.is_empty() {
            "Tool call".to_string()
        } else {
            state.title.clone()
        },
        detail,
    });
}

fn flush_tool_group(sink: &AgentSink, pending: &mut Vec<Block>) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let blocks = std::mem::take(pending);
    send(
        sink,
        AgentEvent::Block {
            block: Block::Group {
                title: "Completed".to_string(),
                blocks,
                collapsed: false,
            },
        },
    )
}

fn first_line(value: &str, max: usize) -> String {
    let line = value.lines().next().unwrap_or("").trim();
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_string()
    }
}

fn send(sink: &AgentSink, event: AgentEvent) -> Result<(), String> {
    sink.send(event)
        .map_err(|_| "the host closed the event stream".to_string())
}

/// A simple common-prefix/suffix diff. Good enough for the ACP `diff` content
/// type; it keeps the card's add/remove/context vocabulary.
fn diff_lines(old: &str, new: &str) -> (Vec<DiffLine>, u32, u32) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut lines = Vec::new();
    for line in &old_lines[..prefix] {
        lines.push(DiffLine::Context((*line).to_string()));
    }
    for line in &old_lines[prefix..old_lines.len() - suffix] {
        lines.push(DiffLine::Remove((*line).to_string()));
    }
    for line in &new_lines[prefix..new_lines.len() - suffix] {
        lines.push(DiffLine::Add((*line).to_string()));
    }
    for line in &old_lines[old_lines.len() - suffix..] {
        lines.push(DiffLine::Context((*line).to_string()));
    }
    let added = new_lines.len().saturating_sub(prefix + suffix) as u32;
    let removed = old_lines.len().saturating_sub(prefix + suffix) as u32;
    (lines, added, removed)
}

enum PromptError {
    Cancelled,
    Message(String),
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptError::Cancelled => f.write_str("cancelled"),
            PromptError::Message(message) => f.write_str(message),
        }
    }
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_keeps_context_and_counts_changes() {
        let (lines, added, removed) = diff_lines("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
        assert!(matches!(lines[0], DiffLine::Context(ref value) if value == "a"));
        assert!(matches!(lines[1], DiffLine::Remove(ref value) if value == "b"));
        assert!(matches!(lines[2], DiffLine::Add(ref value) if value == "B"));
        assert!(matches!(lines[3], DiffLine::Context(ref value) if value == "c"));
    }

    #[test]
    fn args_accept_json_or_whitespace() {
        assert_eq!(parse_args("dsh --profile acp"), ["dsh", "--profile", "acp"]);
        assert_eq!(
            parse_args("[\"dsh\", \"--profile\", \"acp\"]"),
            ["dsh", "--profile", "acp"]
        );
    }

    #[test]
    fn permission_policy_picks_the_requested_kind() {
        let options = vec![
            json!({"optionId": "a", "kind": "allow_once"}),
            json!({"optionId": "r", "kind": "reject_once"}),
        ];
        assert_eq!(pick_permission(&options, true).as_deref(), Some("a"));
        assert_eq!(pick_permission(&options, false).as_deref(), Some("r"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::time::Duration;

    struct AllowOnce;

    impl PermissionPrompter for AllowOnce {
        fn ask(&self, request: PermissionRequest) -> PermissionDecision {
            request
                .options
                .iter()
                .find(|option| option.kind == "allow_once")
                .map(|option| PermissionDecision::Selected(option.id.clone()))
                .unwrap_or(PermissionDecision::Cancelled)
        }
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

    #[test]
    fn fake_acp_maps_text_usage_exec_and_diff() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let fixture = format!("{}/tests/fixtures/fake_acp.py", env!("CARGO_MANIFEST_DIR"));
        let config = AcpConfig {
            program: "python3".to_string(),
            args: vec![fixture],
            cwd: std::env::temp_dir(),
            auto_approve: false,
            name: "test".to_string(),
            debug: false,
            env: Vec::new(),
            auth_method: None,
        };
        let agent = AcpAgent::new(config);
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));

        let mut events = Vec::new();
        loop {
            match stream.recv_timeout(Duration::from_secs(5)) {
                Ok(event) => {
                    let terminal =
                        matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
                    events.push(event);
                    if terminal {
                        break;
                    }
                }
                Err(error) => panic!("ACP stream ended early: {error:?} (events: {events:?})"),
            }
        }

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Started { name } if name == "Fake ACP"
        )));
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Delta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello from ACP.");
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Usage {
                used: 42,
                total: 100
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Block {
                block: Block::Plan { entries }
            } if entries.len() == 2 && entries[0].status == "completed"
        )));
        let grouped: Vec<&Block> = events
            .iter()
            .find_map(|event| match event {
                AgentEvent::Block {
                    block: Block::Group { blocks, .. },
                } => Some(blocks.iter().collect()),
                _ => None,
            })
            .expect("tool calls should be grouped");
        assert!(grouped.iter().any(|block| matches!(
            block,
            Block::Exec { command, .. } if command == "echo hi"
        )));
        assert!(grouped.iter().any(|block| matches!(
            block,
            Block::Diff { path, added, removed, .. }
                if path == "demo.txt" && *added == 1 && *removed == 1
        )));
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
    }

    #[test]
    fn fake_acp_cancel_sends_session_cancel_and_reports_cancelled() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let fixture = format!("{}/tests/fixtures/fake_acp.py", env!("CARGO_MANIFEST_DIR"));
        let config = AcpConfig {
            program: "python3".to_string(),
            args: vec![fixture],
            cwd: std::env::temp_dir(),
            auto_approve: false,
            name: "test".to_string(),
            debug: false,
            env: vec![("FAKE_ACP_CANCEL".to_string(), "1".to_string())],
            auth_method: None,
        };
        let agent = AcpAgent::new(config);
        let (sink, stream) = AgentSink::channel();
        let handle = agent.start("hello", sink);
        std::thread::sleep(Duration::from_millis(250));
        handle.cancel();

        let mut terminal = None;
        for _ in 0..40 {
            match stream.recv_timeout(Duration::from_secs(1)) {
                Ok(event @ (AgentEvent::Finished | AgentEvent::Failed { .. })) => {
                    terminal = Some(event);
                    break;
                }
                Ok(_) => {}
                Err(error) => panic!("cancel stream ended early: {error:?}"),
            }
        }
        assert!(matches!(
            terminal,
            Some(AgentEvent::Failed { ref message }) if message == "Cancelled"
        ));
    }
}
