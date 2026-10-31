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
//! | `EVOBUDDY_ACP_LOG` | append every raw agent message to this file |
//!
//! The plugin is selected by the host when `EVOBUDDY_ACP_COMMAND` is set;
//! otherwise the demo agent remains the answer source.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::{ProtocolVersion, v1 as acp};
use eb_api::model::{Block, DiffLine, PlanEntry};
use eb_api::{
    AgentEvent, AgentSink, AnswerHandle, AnswerSource, DenyAll, EbPlugin, PermissionDecision,
    PermissionOption, PermissionPrompter, PermissionRequest, Ui,
};
use serde_json::{json, Value};

/// Grok's private "a session row changed" notification. Its upserted row carries
/// `activity`: `working` for as long as the session has running work — including
/// work a subagent is doing — and `idle` when it really stops.
const ACTIVITY_METHOD: &str = "_x.ai/sessions/changed";

/// How often a bounded wait re-checks cancellation before the next message.
const POLL: Duration = Duration::from_millis(250);

/// How long a single handshake request (`initialize`, `session/new`, …) may go
/// unanswered before the session is treated as broken.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

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
    pub tail: TailPolicy,
    /// Append every raw agent message to this file. Debugging an agent that
    /// ends its turn early needs the transcript, not a reconstruction: which
    /// signal arrived, in what order, and what it said.
    pub log: Option<PathBuf>,
}

/// How long a turn stays open after `session/prompt` answers.
///
/// An agent that hands off to background work (Grok's `deep-research`, a
/// subagent) returns `stopReason=end_turn` first and keeps streaming
/// `session/update` afterwards. Ending the turn at the response would drop that
/// continuation, so the prompt loop keeps draining the session.
///
/// How long it waits depends on what the agent has told us about itself. An
/// agent that reports session activity (`_x.ai/sessions/changed`) says exactly
/// when it stops, so silence is not evidence of an end: while it says it is
/// working, only a long safety window applies. An agent that reports nothing is
/// all we have silence for, and the short probe window keeps its turns from
/// hanging around after the answer.
#[derive(Clone, Copy, Debug)]
pub struct TailPolicy {
    /// Silence that ends the turn when the agent has given us no reason to
    /// believe it is still busy. Also the closing probe after an `idle` signal.
    pub first: Duration,
    /// Silence tolerated once a continuation message has arrived.
    pub quiet: Duration,
    /// Silence tolerated while the agent says the session is still working.
    /// Kept under the inactivity watchdog (`EVOBUDDY_ACP_TIMEOUT_SECS`, 180s)
    /// so the turn always ends before the child would be killed.
    pub busy: Duration,
    /// Hard cap on the whole continuation window, for agents that report no
    /// activity. A session the agent itself calls `working` is not capped —
    /// `busy` and the watchdog are what end it.
    pub max: Duration,
}

impl Default for TailPolicy {
    fn default() -> Self {
        TailPolicy {
            first: Duration::from_secs(2),
            quiet: Duration::from_secs(40),
            busy: Duration::from_secs(120),
            max: Duration::from_secs(900),
        }
    }
}

impl TailPolicy {
    /// Env overrides, so a machine whose agent goes quiet for longer than two
    /// seconds before it picks up background work can be retuned without a
    /// rebuild.
    pub fn from_env() -> Self {
        let default = Self::default();
        let secs = |key: &str, fallback: Duration| {
            std::env::var(key)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(fallback)
        };
        TailPolicy {
            first: secs("EVOBUDDY_ACP_TAIL_FIRST_SECS", default.first),
            quiet: secs("EVOBUDDY_ACP_TAIL_QUIET_SECS", default.quiet),
            busy: secs("EVOBUDDY_ACP_TAIL_BUSY_SECS", default.busy),
            max: secs("EVOBUDDY_ACP_TAIL_MAX_SECS", default.max),
        }
    }
}

impl AcpConfig {
    /// `EVOBUDDY_ACP_LOG`, with an empty value treated as unset.
    pub fn log_from_env() -> Option<PathBuf> {
        std::env::var("EVOBUDDY_ACP_LOG")
            .ok()
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
    }

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
            tail: TailPolicy::from_env(),
            log: Self::log_from_env(),
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

/// What an agent answered during an [`AcpAgent::probe`] handshake.
#[derive(Clone, Debug)]
pub struct AcpProbe {
    pub agent_name: String,
    /// The `agentCapabilities` object from `initialize`: `loadSession`,
    /// `promptCapabilities`, `sessionCapabilities`, …
    pub capabilities: Value,
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
    pid: Option<u32>,
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

    /// Spawn the agent, run `initialize` + `session/new`, then drop the session.
    /// No `session/prompt` is sent, so this costs no model tokens: it answers
    /// "does this executable speak ACP, and what does it advertise?" for the
    /// settings page and the `EB_ACP_PROBE` harness.
    pub fn probe(&self, workspace: &std::path::Path) -> Result<AcpProbe, String> {
        let session = AcpSession::launch(&self.config, Arc::new(DenyAll), workspace)?;
        let probe = AcpProbe {
            agent_name: session.agent_name.clone(),
            capabilities: session.capabilities.clone(),
        };
        // `Drop` kills the child, so a probe that succeeds never leaves a
        // daemon-shaped process behind.
        drop(session);
        Ok(probe)
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
        let exited = slot
            .as_mut()
            .and_then(|session| session.child.try_wait().ok().flatten())
            .is_some();
        if exited || slot.as_ref().is_some_and(|session| session.cwd != workspace) {
            // A previous turn can leave a dead child (agent crash or a stale
            // session); relaunch rather than writing into a closed pipe.
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
                    cancel.pid = Some(session.child.id());
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
        let done = Arc::new(AtomicBool::new(false));
        {
            let cancel = self.cancel.clone();
            let done = done.clone();
            let progress = session.progress.clone();
            let limit = std::env::var("EVOBUDDY_ACP_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(180);
            std::thread::spawn(move || {
                // Silence, not wall clock: a research turn that keeps producing
                // output is progress and must not be killed mid-flight.
                let mut quiet_since = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    if done.load(Ordering::SeqCst) {
                        return;
                    }
                    if progress.swap(false, Ordering::SeqCst) {
                        quiet_since = Instant::now();
                    }
                    if quiet_since.elapsed() < Duration::from_secs(limit) {
                        continue;
                    }
                    let state = lock(&cancel);
                    if let Some(pid) = state.pid {
                        let _ = std::process::Command::new("kill")
                            .arg("-TERM")
                            .arg(pid.to_string())
                            .status();
                        std::thread::sleep(Duration::from_secs(3));
                        if !done.load(Ordering::SeqCst) {
                            // An agent that ignores SIGTERM still holds the pipe
                            // open; SIGKILL guarantees the turn ends.
                            let _ = std::process::Command::new("kill")
                                .arg("-KILL")
                                .arg(pid.to_string())
                                .status();
                        }
                    }
                    return;
                }
            });
        }
        let result = session.prompt(&prompt, &sink, &cancelled, self.config.tail);
        done.store(true, Ordering::SeqCst);
        match result {
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

/// Convert EvoBuddy connector files into ACP `mcpServers` entries.
///
/// Each `~/.evobuddy/connectors/*.json` may be either one server object or a
/// Claude-style `{ "mcpServers": { "name": { ... } } }` wrapper. Stdio servers
/// use `command`/`args`/`env`; remote servers use `url`/`headers`.
fn read_mcp_servers(root: &std::path::Path) -> Vec<Value> {
    let mut servers = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return servers };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
        let default_name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "connector".into());
        if let Some(group) = value.get("mcpServers").and_then(Value::as_object) {
            for (name, config) in group {
                if let Some(server) = mcp_server(name, config) {
                    servers.push(server);
                }
            }
        } else {
            let name = value.get("name").and_then(Value::as_str).unwrap_or(&default_name);
            if let Some(server) = mcp_server(name, &value) {
                servers.push(server);
            }
        }
    }
    servers
}

/// The Grok Build workaround, in the typed shape: an agent that rejects stdio
/// MCP servers in `session/new` still gets the conversation on an empty list.
fn retry_session_new(
    session: &mut AcpSession,
    cwd: &std::path::Path,
    error: &str,
    debug: bool,
) -> Result<acp::NewSessionResponse, String> {
    if debug {
        eprintln!("acp: session/new rejected MCP servers: {error}; retrying without them");
    }
    session.typed_request("session/new", acp::NewSessionRequest::new(cwd.to_path_buf()))
}

/// The same workaround, in the raw shape used when connector definitions do
/// not fit the typed schema.
fn retry_session_new_raw(
    session: &mut AcpSession,
    cwd: &str,
    error: &str,
    debug: bool,
) -> Result<Value, String> {
    if debug {
        eprintln!("acp: session/new rejected MCP servers: {error}; retrying without them");
    }
    session.request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))
}

fn mcp_server(name: &str, config: &Value) -> Option<Value> {
    if let Some(command) = config.get("command").and_then(Value::as_str) {
        if command.trim().is_empty() {
            return None;
        }
        let mut server = json!({
            "name": name,
            "command": command,
            "args": config.get("args").cloned().unwrap_or_else(|| json!([])),
        });
        if let Some(env) = config.get("env").filter(|value| value.is_object()) {
            server["env"] = env.clone();
        }
        return Some(server);
    }
    if let Some(url) = config.get("url").and_then(Value::as_str) {
        if url.trim().is_empty() {
            return None;
        }
        let mut server = json!({ "name": name, "url": url });
        if let Some(headers) = config.get("headers").filter(|value| value.is_object()) {
            server["headers"] = headers.clone();
        }
        return Some(server);
    }
    None
}

fn mcp_servers() -> Vec<Value> {
    let root = std::env::var("EB_CONNECTORS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home).join(".evobuddy/connectors")
        });
    read_mcp_servers(&root)
}

struct AcpSession {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    /// Decoded agent messages. A dedicated thread owns stdout so that nothing
    /// is lost between the phases of a turn — see [`AcpSession::spawn_reader`].
    rx: Receiver<Result<Value, String>>,
    progress: Arc<AtomicBool>,
    next_id: u64,
    session_id: String,
    cwd: std::path::PathBuf,
    agent_name: String,
    /// What `initialize` answered under `agentCapabilities`, kept only so
    /// [`AcpAgent::probe`] can report it.
    capabilities: Value,
    auto_approve: bool,
    permissions: Arc<dyn PermissionPrompter>,
    debug: bool,
    /// What this session last said about itself. The reader thread keeps it live
    /// between turns, so it is the agent's current word rather than this turn's:
    /// work left running when a turn gave up is never announced again (the row
    /// already says `working`), and an `idle` that arrived while nobody was
    /// listening says the session really did finish.
    activity: Option<bool>,
    /// Agent-spawned terminals from the `terminal/*` client methods, keyed by
    /// the numeric id this side assigned.
    terminals: HashMap<u64, Terminal>,
}

/// One agent-spawned terminal from ACP's `terminal/*` client methods.
///
/// The agent owns the lifecycle: create → poll output → wait for exit →
/// release. Output is capped so a chatty process cannot grow memory without
/// bound; `truncated` tells the agent earlier bytes were dropped.
struct Terminal {
    /// `Some` while the process runs; the reaper thread `take()`s it so
    /// `Child::wait()` never holds the lock (a kill must not block on it).
    process: Arc<Mutex<Option<Child>>>,
    output: Arc<Mutex<String>>,
    /// The reaped status, per `TerminalExitStatus` (exit code or signal).
    exit: Arc<Mutex<Option<acp::TerminalExitStatus>>>,
    /// `outputByteLimit` from `terminal/create` (agent's cap or the default).
    cap: usize,
}

/// Output kept per terminal; the head is trimmed once this cap is exceeded.
const TERMINAL_OUTPUT_CAP: usize = 1_000_000;

/// Trim a terminal output buffer to [`TERMINAL_OUTPUT_CAP`], keeping the tail.
/// Returns whether earlier bytes were dropped.
fn trim_terminal_output(output: &mut String) -> bool {
    if output.len() <= TERMINAL_OUTPUT_CAP {
        return false;
    }
    let overflow = output.len() - TERMINAL_OUTPUT_CAP;
    let split = output
        .char_indices()
        .map(|(index, _)| index)
        .find(|&index| index >= overflow)
        .unwrap_or(output.len());
    output.drain(..split);
    true
}

/// Outcome of one bounded wait for the agent.
enum Received {
    Message(Value),
    Timeout,
    Closed(String),
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
        let stdout = child.stdout.take().ok_or("agent stdout was not piped")?;
        if config.debug {
            if let Some(stderr) = child.stderr.take() {
                std::thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        eprintln!("acp: {line}");
                    }
                });
            }
        }

        let progress = Arc::new(AtomicBool::new(false));
        let rx = spawn_reader(stdout, progress.clone(), config.debug, config.log.clone());
        let mut session = AcpSession {
            child,
            stdin,
            rx,
            progress,
            next_id: 1,
            session_id: String::new(),
            cwd: cwd.to_path_buf(),
            agent_name: config.name.clone(),
            capabilities: Value::Null,
            auto_approve: config.auto_approve,
            permissions,
            debug: config.debug,
            activity: None,
            terminals: HashMap::new(),
        };

        if config.debug {
            eprintln!("acp: initialize cwd={}", cwd.display());
        }
        // Typed handshake from the official SDK: the capability object is what
        // makes agents offer the fs/terminal client methods this side answers
        // in `respond_request`.
        let init_request = acp::InitializeRequest::new(ProtocolVersion::V1)
            .client_capabilities(
                acp::ClientCapabilities::new().fs(
                    acp::FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true),
                ).terminal(true),
            )
            .client_info(acp::Implementation::new(
                "evobuddy",
                env!("CARGO_PKG_VERSION"),
            )
            .title("EvoBuddy"));
        let init_response: acp::InitializeResponse =
            session.typed_request("initialize", init_request)?;
        if init_response.protocol_version != ProtocolVersion::V1 {
            return Err(format!(
                "ACP version {} is not supported (need v1)",
                init_response.protocol_version
            ));
        }
        if let Some(method) = &config.auth_method {
            session.request("authenticate", json!({ "methodId": method }))?;
        }
        if let Some(agent_info) = &init_response.agent_info {
            if let Some(title) = agent_info.title.as_deref().filter(|v| !v.is_empty()) {
                session.agent_name = title.to_string();
            } else {
                session.agent_name = agent_info.name.clone();
            }
        }
        session.capabilities = serde_json::to_value(&init_response.agent_capabilities)
            .unwrap_or(Value::Null);

        let cwd = cwd.to_string_lossy();
        let servers = mcp_servers();
        let cwd_path = PathBuf::from(&*cwd);
        if config.debug {
            eprintln!("acp: session/new with {} MCP server(s)", servers.len());
        }
        // Connector files may carry fields outside the typed schema; convert
        // when they fit, and fall back to the raw shape when they do not.
        let typed_servers: Option<Vec<acp::McpServer>> =
            serde_json::from_value(json!(servers)).ok();
        let new_session: acp::NewSessionResponse = match typed_servers {
            Some(typed) => {
                let mut new_session_request = acp::NewSessionRequest::new(cwd_path.clone());
                new_session_request.mcp_servers = typed;
                match session.typed_request("session/new", new_session_request) {
                    Ok(value) => value,
                    Err(error) if !servers.is_empty() => retry_session_new(&mut session, &cwd_path, &error, config.debug)?,
                    Err(error) => return Err(error),
                }
            }
            None => {
                let value = match session.request(
                    "session/new",
                    json!({ "cwd": cwd, "mcpServers": servers }),
                ) {
                    Ok(value) => value,
                    Err(error) if !servers.is_empty() => {
                        retry_session_new_raw(&mut session, &cwd, &error, config.debug)?
                    }
                    Err(error) => return Err(error),
                };
                serde_json::from_value(value)
                    .map_err(|error| format!("session/new: unexpected response shape: {error}"))?
            }
        };
        let session_id = new_session.session_id.0.to_string();
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
        tail: TailPolicy,
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

        // Anything still queued was produced before this request reached the
        // agent — the tail of a turn that gave up while the agent kept working.
        // Its requests still get answered (the agent would block on them), but
        // attributing its text to this turn would glue the previous answer onto
        // the new question. Its `activity` flips are the agent's current word on
        // the session, so those are kept.
        self.discard_backlog();

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

            let message = match self.next(POLL) {
                Received::Timeout => continue,
                Received::Closed(error) => return Err(PromptError::Message(error)),
                Received::Message(message) => message,
            };
            self.note_activity(&message);
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
                // Typed first; an agent sending a stop reason outside the
                // schema falls back to the raw string.
                let stop = match serde_json::from_value::<acp::PromptResponse>(
                    message.get("result").cloned().unwrap_or(Value::Null),
                ) {
                    Ok(response) => match response.stop_reason {
                        acp::StopReason::EndTurn => "end_turn".to_string(),
                        acp::StopReason::Cancelled => "cancelled".to_string(),
                        other => format!("{other:?}"),
                    },
                    Err(_) => message
                        .pointer("/result/stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn")
                        .to_string(),
                };
                flush_tool_group(sink, &mut pending_tools).map_err(PromptError::Message)?;
                if stop == "cancelled" {
                    return Err(PromptError::Cancelled);
                }
                if stop != "end_turn" && self.debug {
                    eprintln!("acp: stopReason={stop}");
                }
                return self.drain_tail(sink, cancelled, &mut tools, &mut pending_tools, tail);
            }

            let Some(method) = message.get("method").and_then(Value::as_str) else {
                continue;
            };
            if message.get("id").is_some() {
                self.respond_request(&message)
                    .map_err(PromptError::Message)?;
                continue;
            }
            if is_update_notification(method) {
                map_update(&message["params"]["update"], sink, &mut tools, &mut pending_tools)
                    .map_err(PromptError::Message)?;
            }
        }
    }

    /// Keep the turn open while the agent works past its own response.
    ///
    /// The agent's own word outranks the clock. Grok keeps the session
    /// `working` for the whole of a background hand-off — in a captured run it
    /// stayed `working` for 1403s across 60 provider retries, gaps of 35s with
    /// nothing on the pipe — and flips it `idle` in the same millisecond the
    /// prompt really ends. So while it says `working` the turn stays open on a
    /// long safety window, and the flip to `idle` closes it after a short probe
    /// for stragglers. An agent that never reports activity gets the old
    /// silence-probe behaviour, so its turns still end promptly.
    fn drain_tail(
        &mut self,
        sink: &AgentSink,
        cancelled: &AtomicBool,
        tools: &mut HashMap<String, ToolState>,
        pending_tools: &mut Vec<Block>,
        tail: TailPolicy,
    ) -> Result<(), PromptError> {
        let started = Instant::now();
        let mut continuing = false;
        // Output that landed *after* the agent said `idle`. None is expected, so
        // the flip only gets a short probe; but if an agent really does stream
        // past its own `idle`, that stream is the answer and the normal
        // continuation window applies to it.
        let mut after_idle = 0usize;
        loop {
            if cancelled.load(Ordering::SeqCst) {
                return Err(PromptError::Cancelled);
            }
            // The cap bounds a turn we are only guessing at. A session the agent
            // itself calls `working` is bounded by `busy` and, if it really dies,
            // by the inactivity watchdog.
            if self.activity != Some(true) && started.elapsed() >= tail.max {
                if self.debug {
                    eprintln!("acp: tail window capped after {:?}", started.elapsed());
                }
                return Ok(());
            }
            let wait = match self.activity {
                // The agent says it is running: silence proves nothing, so only
                // the long safety window applies.
                Some(true) => tail.busy,
                Some(false) if after_idle == 0 => tail.first,
                Some(false) => tail.quiet,
                None if continuing => tail.quiet,
                None => tail.first,
            };
            let message = match self.next(wait) {
                Received::Timeout => {
                    if self.debug {
                        eprintln!(
                            "acp: tail closed after {wait:?} of silence (activity={:?}, continuing={continuing})",
                            self.activity
                        );
                    }
                    return Ok(());
                }
                Received::Closed(error) => return Err(PromptError::Message(error)),
                Received::Message(message) => message,
            };
            self.note_activity(&message);
            let Some(method) = message.get("method").and_then(Value::as_str) else {
                continue;
            };
            if message.get("id").is_some() {
                self.respond_request(&message)
                    .map_err(PromptError::Message)?;
                continue;
            }
            // A bare activity flip is not output; calling the turn "still
            // working" because of one would be true but tells the user nothing.
            if method == ACTIVITY_METHOD {
                continue;
            }
            if self.activity == Some(false) {
                after_idle += 1;
            }
            if !continuing {
                continuing = true;
                if self.debug {
                    eprintln!("acp: agent kept working after the response");
                }
                let _ = send(
                    sink,
                    AgentEvent::Status {
                        text: "Agent is still working".to_string(),
                    },
                );
            }
            if is_update_notification(method) {
                let update = &message["params"]["update"];
                // Grok's own end-of-turn notice rides the same private envelope,
                // but it is fired with the response — `turn_completed` and
                // `prompt_complete` both land within milliseconds of it — so in
                // the tail it is only evidence about *how* work that is already
                // over failed. Ending the turn on its `end_turn` would cut the
                // continuation off at the first chunk; the session's `activity`
                // flip is what says the background work is done.
                if update.get("sessionUpdate").and_then(Value::as_str) == Some("turn_completed") {
                    let stop = update
                        .get("stop_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn");
                    if self.debug {
                        eprintln!("acp: tail turn_completed stop_reason={stop}");
                    }
                    if stop == "cancelled" {
                        return Err(PromptError::Cancelled);
                    }
                    if stop == "error" {
                        let why = update
                            .get("agent_result")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                            .unwrap_or("the agent reported an error");
                        return Err(PromptError::Message(format!(
                            "agent turn failed after the response: {why}"
                        )));
                    }
                    continue;
                }
                map_update(update, sink, tools, pending_tools)
                    .map_err(PromptError::Message)?;
            }
        }
    }

    /// Send a request whose params/responses are typed by the official SDK.
    /// Serialization/deserialization stay on this side so the sync transport,
    /// timeouts and mid-request client-request answering all keep working.
    fn typed_request<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, String> {
        let params = serde_json::to_value(&params)
            .map_err(|error| format!("{method}: params failed to serialize: {error}"))?;
        let value = self.request(method, params)?;
        serde_json::from_value(value)
            .map_err(|error| format!("{method}: unexpected response shape: {error}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let message = match self.next(POLL) {
                Received::Timeout => {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "{method} timed out after {}s of silence",
                            REQUEST_TIMEOUT.as_secs()
                        ));
                    }
                    continue;
                }
                Received::Closed(error) => return Err(error),
                Received::Message(message) => message,
            };
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
        if method == "session/request_permission" {            let raw_options = message
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
                Some(option_id) => {
                    let outcome =
                        acp::SelectedPermissionOutcome::new(acp::PermissionOptionId::new(option_id.clone()));
                    acp::RequestPermissionResponse::new(
                        acp::RequestPermissionOutcome::Selected(outcome),
                    )
                }
                None => {
                    acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Cancelled)
                }
            };
            let result = serde_json::to_value(&outcome).unwrap_or(Value::Null);
            return self.write(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }));
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // Typed dispatch through the official schema: an unknown method or a
        // params shape outside v1 lands on the unsupported error either way.
        let served: Result<Value, String> = match method {
            "fs/read_text_file" => serde_json::from_value::<acp::ReadTextFileRequest>(params)
                .map_err(|error| format!("fs/read_text_file: {error}"))
                .and_then(|request| fs_read(&self.cwd, request)),
            "fs/write_text_file" => serde_json::from_value::<acp::WriteTextFileRequest>(params)
                .map_err(|error| format!("fs/write_text_file: {error}"))
                .and_then(|request| fs_write(&self.cwd, request)),
            "terminal/create" => serde_json::from_value::<acp::CreateTerminalRequest>(params)
                .map_err(|error| format!("terminal/create: {error}"))
                .and_then(|request| terminal_create(&self.cwd, request, &mut self.terminals)),
            "terminal/output" => serde_json::from_value::<acp::TerminalOutputRequest>(params)
                .map_err(|error| format!("terminal/output: {error}"))
                .and_then(|request| terminal_output(request, &mut self.terminals)),
            "terminal/wait_for_exit" => {
                serde_json::from_value::<acp::WaitForTerminalExitRequest>(params)
                    .map_err(|error| format!("terminal/wait_for_exit: {error}"))
                    .and_then(|request| terminal_wait(request, &mut self.terminals))
            }
            "terminal/kill" => serde_json::from_value::<acp::KillTerminalRequest>(params)
                .map_err(|error| format!("terminal/kill: {error}"))
                .and_then(|request| terminal_kill(request, &mut self.terminals)),
            "terminal/release" => serde_json::from_value::<acp::ReleaseTerminalRequest>(params)
                .map_err(|error| format!("terminal/release: {error}"))
                .and_then(|request| terminal_release(request, &mut self.terminals)),
            _ => Err(format!("unsupported client method: {method}")),
        };
        match served {
            Ok(result) => self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result })),
            Err(error) => self.write(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": error }
            })),
        }
    }

    fn write(&mut self, value: &Value) -> Result<(), String> {
        write_json(&self.stdin, value)
    }

    fn next(&mut self, timeout: Duration) -> Received {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(value)) => Received::Message(value),
            Ok(Err(error)) => Received::Closed(error),
            Err(mpsc::RecvTimeoutError::Timeout) => Received::Timeout,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Received::Closed("ACP agent closed stdout".to_string())
            }
        }
    }

    /// Record what the agent says about its own session.
    ///
    /// Grok upserts the session row with `activity` `working` or `idle` around
    /// every prompt, and that is the only signal which spans background work —
    /// the spec response, `turn_completed` and `prompt_complete` all fire
    /// together. An agent that reports nothing leaves `activity` at `None`, and
    /// the tail then falls back to the silence windows.
    fn note_activity(&mut self, message: &Value) {
        if message.get("method").and_then(Value::as_str) != Some(ACTIVITY_METHOD) {
            return;
        }
        let Some(sessions) = message
            .pointer("/params/upserted")
            .and_then(Value::as_array)
        else {
            return;
        };
        for session in sessions {
            if session.get("sessionId").and_then(Value::as_str) != Some(self.session_id.as_str()) {
                continue;
            }
            let working = match session.get("activity").and_then(Value::as_str) {
                Some("working") => true,
                Some("idle") => false,
                other => {
                    if self.debug {
                        eprintln!("acp: unrecognised session activity: {other:?}");
                    }
                    continue;
                }
            };
            if self.debug && self.activity != Some(working) {
                eprintln!(
                    "acp: session activity {}",
                    if working { "working" } else { "idle" }
                );
            }
            self.activity = Some(working);
        }
    }

    /// Drop what the reader queued while no turn was listening.
    ///
    /// A turn that gave up while the agent kept working leaves that work in the
    /// channel; the next turn would otherwise stream it into its own answer.
    /// `activity` is the exception: it describes the session, not a turn, so the
    /// newest queued flip is what this prompt starts from.
    fn discard_backlog(&mut self) {
        let mut dropped = 0usize;
        let mut answered = 0usize;
        loop {
            let message = match self.rx.try_recv() {
                Ok(Ok(message)) => message,
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => break,
            };
            self.note_activity(&message);
            // Requests still have to be answered: an agent blocked on a call it
            // made between turns would never get to this prompt.
            if message.get("id").is_some() && message.get("method").is_some() {
                let _ = self.respond_request(&message);
                answered += 1;
                continue;
            }
            dropped += 1;
        }
        if self.debug && dropped + answered > 0 {
            eprintln!("acp: cleared the backlog: {dropped} message(s) dropped, {answered} answered");
        }
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drain the agent's stdout on its own thread.
///
/// The pipe must be read continuously: an agent that finishes a turn and then
/// keeps working would otherwise block on a full buffer, and its late
/// `session/update` messages — the answer the user is waiting for — would never
/// be read at all.
///
/// With `log` set, every raw line is appended to that file first. Deciding when
/// a turn ends needs the transcript — which signal arrived, in what order — and
/// a driver that reproduces the turn costs the user tokens a normal run already
/// spends.
fn spawn_reader(
    stdout: ChildStdout,
    progress: Arc<AtomicBool>,
    debug: bool,
    log: Option<PathBuf>,
) -> Receiver<Result<Value, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("evobuddy-acp-read".to_string())
        .spawn(move || {
            // Opened once, and a broken path is said out loud rather than
            // turning the transcript off mid-run.
            let mut transcript = open_transcript(log);
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(Err("ACP agent closed stdout".to_string()));
                        return;
                    }
                    Ok(_) => {
                        progress.store(true, Ordering::SeqCst);
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(file) = &mut transcript {
                            let _ = writeln!(file, "{:.3}\t{line}", wall_clock());
                        }
                        match serde_json::from_str::<Value>(line) {
                            Ok(value) => {
                                if tx.send(Ok(value)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                let message = format!("invalid ACP JSON: {error}: {line}");
                                if debug {
                                    eprintln!("acp: {message}");
                                }
                                if tx.send(Err(message)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(Err(format!("read from ACP agent: {error}")));
                        return;
                    }
                }
            }
        })
        .expect("spawn ACP reader");
    rx
}

/// The transcript log from `EVOBUDDY_ACP_LOG`, or nothing if it cannot be
/// opened — a bad path must not cost the user their turn.
fn open_transcript(path: Option<PathBuf>) -> Option<std::fs::File> {
    let path = path?;
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!(
                "acp: transcript log {} is not writable: {error}",
                path.display()
            );
            None
        }
    }
}

fn wall_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0)
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

/// Resolve an agent-supplied path against the session cwd.
///
/// ACP paths are absolute; a relative path is interpreted against the
/// workspace the session was opened on, which is also what `terminal/create`
/// uses when the agent sends no cwd.
fn resolve_path(cwd: &std::path::Path, raw: &str) -> PathBuf {
    let path = std::path::Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// `fs/read_text_file`: `{path, line?, limit?}` → `{content}`.
///
/// `line` is 1-based like the ACP spec says; `limit` caps the returned lines.
fn fs_read(cwd: &std::path::Path, request: acp::ReadTextFileRequest) -> Result<Value, String> {
    let path = resolve_path(cwd, &request.path.to_string_lossy());
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("fs/read_text_file: {}: {error}", path.display()))?;
    let content = match (request.line, request.limit) {
        (None, None) => text,
        (line, limit) => {
            let start = line.unwrap_or(1).saturating_sub(1) as usize;
            let limit = limit.map(|value| value as usize).unwrap_or(usize::MAX);
            text.lines()
                .skip(start)
                .take(limit)
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    serde_json::to_value(acp::ReadTextFileResponse::new(content))
        .map_err(|error| format!("fs/read_text_file: {error}"))
}

/// `fs/write_text_file`: `{path, content}` → null result.
fn fs_write(cwd: &std::path::Path, request: acp::WriteTextFileRequest) -> Result<Value, String> {
    let path = resolve_path(cwd, &request.path.to_string_lossy());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("fs/write_text_file: {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, &request.content)
        .map_err(|error| format!("fs/write_text_file: {}: {error}", path.display()))?;
    serde_json::to_value(acp::WriteTextFileResponse::new())
        .map_err(|error| format!("fs/write_text_file: {error}"))
}

/// Map an agent-supplied wire id (a `TerminalId` string) back to the numeric
/// key this side generated in `terminal/create`.
fn terminal_key(id: &acp::TerminalId, method: &str) -> Result<u64, String> {
    id.0
        .parse::<u64>()
        .map_err(|_| format!("{method}: unknown terminalId {}", id.0))
}

/// `terminal/create`: spawn `{command, args?, cwd?, env?}` and start draining
/// its merged output. Returns `{terminalId}`; an agent-sent `outputByteLimit`
/// is honored, with this side's own cap as the default.
fn terminal_create(
    cwd: &std::path::Path,
    request: acp::CreateTerminalRequest,
    terminals: &mut HashMap<u64, Terminal>,
) -> Result<Value, String> {
    let dir = request
        .cwd
        .map(|raw| resolve_path(cwd, &raw.to_string_lossy()))
        .unwrap_or_else(|| cwd.to_path_buf());
    let mut process = Command::new(&request.command);
    for arg in &request.args {
        process.arg(arg);
    }
    for entry in &request.env {
        process.env(&entry.name, &entry.value);
    }
    // The agent may ask for a tighter cap than the safety default; never wider.
    let cap = request
        .output_byte_limit
        .map(|value| value.min(TERMINAL_OUTPUT_CAP as u64))
        .unwrap_or(TERMINAL_OUTPUT_CAP as u64) as usize;
    process
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.spawn().map_err(|error| {
        format!("terminal/create: {}: {error}", request.command)
    })?;

    let output: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let exit: Arc<Mutex<Option<acp::TerminalExitStatus>>> = Arc::new(Mutex::new(None));
    let stdout = child.stdout.take().ok_or("terminal/create: no stdout")?;
    let stderr = child.stderr.take().ok_or("terminal/create: no stderr")?;
    for pipe in [
        Box::new(BufReader::new(stdout)) as Box<dyn BufRead + Send>,
        Box::new(BufReader::new(stderr)) as Box<dyn BufRead + Send>,
    ] {
        let output = output.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(pipe);
            for chunk in reader.split(b'\n') {
                let mut buffer = output.lock().unwrap();
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        buffer.push('\n');
                    }
                    // A partial trailing line (no newline yet) still lands in
                    // the buffer; decode lossily like a terminal would.
                    Err(_) => break,
                }
            }
        });
    }
    // Reap the process once it exits so `wait_for_exit` is an answer, not a
    // hang, even if the agent never asks. The reaper polls with `try_wait`,
    // holding the lock only briefly, so `terminal/kill` can always get in.
    let process = Arc::new(Mutex::new(Some(child)));
    let exit_status = exit.clone();
    let reaped = process.clone();
    std::thread::spawn(move || loop {
        let outcome = {
            let mut slot = reaped.lock().unwrap();
            match slot.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        *slot = None;
                        Some(status)
                    }
                    Ok(None) => None,
                    Err(_) => {
                        *slot = None;
                        Some(std::process::ExitStatus::default())
                    }
                },
                None => break,
            }
        };
        if let Some(status) = outcome {
            *exit_status.lock().unwrap() = Some(TerminalExit::from_process_status(status));
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    });

    let id = terminals.keys().copied().max().unwrap_or(0) + 1;
    terminals.insert(
        id,
        Terminal {
            process,
            output,
            exit,
            cap,
        },
    );
    serde_json::to_value(acp::CreateTerminalResponse::new(acp::TerminalId::from(
        id.to_string(),
    )))
    .map_err(|error| format!("terminal/create: {error}"))
}

/// Reap outcome -> the schema's `TerminalExitStatus` (code, or signal name).
struct TerminalExit;
impl TerminalExit {
    #[cfg(unix)]
    fn from_process_status(
        status: std::process::ExitStatus,
    ) -> acp::TerminalExitStatus {
        if let Some(code) = status.code() {
            return acp::TerminalExitStatus::new().exit_code(u32::try_from(code).unwrap_or(0));
        }
        let signal = std::os::unix::process::ExitStatusExt::signal(&status)
            .map(|signal| signal.to_string());
        acp::TerminalExitStatus::new().signal(signal)
    }

    #[cfg(not(unix))]
    fn from_process_status(
        status: std::process::ExitStatus,
    ) -> acp::TerminalExitStatus {
        acp::TerminalExitStatus::new()
            .exit_code(u32::try_from(status.code().unwrap_or(0)).unwrap_or(0))
    }
}

/// Look up a terminal the agent refers to, with a method-specific error.
fn terminal_ref<'a>(
    id: &acp::TerminalId,
    terminals: &'a mut HashMap<u64, Terminal>,
    method: &str,
) -> Result<&'a mut Terminal, String> {
    let key = terminal_key(id, method)?;
    terminals
        .get_mut(&key)
        .ok_or_else(|| format!("{method}: unknown terminalId {}", id.0))
}

/// `terminal/output`: `{terminalId}` → `{output, truncated, exitStatus?}`,
/// trimmed to the create-time cap (the agent's or the 1 MB default).
fn terminal_output(
    request: acp::TerminalOutputRequest,
    terminals: &mut HashMap<u64, Terminal>,
) -> Result<Value, String> {
    let terminal = terminal_ref(&request.terminal_id, terminals, "terminal/output")?;
    let mut buffer = terminal.output.lock().unwrap();
    // Honor the create-time cap (`outputByteLimit` or the 1 MB default):
    // trim from the head, keeping whole characters at the boundary.
    let mut cap = terminal.cap;
    if buffer.len() > cap {
        cap = buffer
            .char_indices()
            .map(|(index, _)| index)
            .find(|&index| index >= buffer.len() - cap)
            .unwrap_or(buffer.len());
        buffer.drain(..cap);
    }
    let truncated = trim_terminal_output(&mut buffer);
    let exit = terminal.exit.lock().unwrap().clone();
    let mut response = acp::TerminalOutputResponse::new(buffer.clone(), truncated);
    if let Some(exit) = exit {
        response = response.exit_status(exit);
    }
    serde_json::to_value(response).map_err(|error| format!("terminal/output: {error}"))
}

/// `terminal/wait_for_exit`: blocks until the process is reaped.
fn terminal_wait(
    request: acp::WaitForTerminalExitRequest,
    terminals: &mut HashMap<u64, Terminal>,
) -> Result<Value, String> {
    let terminal = terminal_ref(&request.terminal_id, terminals, "terminal/wait_for_exit")?;
    loop {
        if let Some(exit) = terminal.exit.lock().unwrap().clone() {
            return serde_json::to_value(acp::WaitForTerminalExitResponse::new(exit))
                .map_err(|error| format!("terminal/wait_for_exit: {error}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `terminal/kill`: stop the process; exit status lands via the reaper thread.
fn terminal_kill(
    request: acp::KillTerminalRequest,
    terminals: &mut HashMap<u64, Terminal>,
) -> Result<Value, String> {
    let terminal = terminal_ref(&request.terminal_id, terminals, "terminal/kill")?;
    if let Some(child) = terminal.process.lock().unwrap().as_mut() {
        let _ = child.kill();
    }
    serde_json::to_value(acp::KillTerminalResponse::new())
        .map_err(|error| format!("terminal/kill: {error}"))
}

/// `terminal/release`: kill if still running and forget the terminal.
fn terminal_release(
    request: acp::ReleaseTerminalRequest,
    terminals: &mut HashMap<u64, Terminal>,
) -> Result<Value, String> {
    let key = terminal_key(&request.terminal_id, "terminal/release")?;
    if let Some(terminal) = terminals.remove(&key) {
        if let Some(child) = terminal.process.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
    serde_json::to_value(acp::ReleaseTerminalResponse::new())
        .map_err(|error| format!("terminal/release: {error}"))
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

/// Whether a notification carries an update in `params.update`.
///
/// Grok sends most of its session updates — including all 52 `retry_state`
/// notices in the captured outage transcript — through a private
/// `_x.ai/session_notification` method with the spec's payload shape. Matching
/// only `session/update` makes those invisible, which is exactly the "silent
/// hang" the status line exists to avoid.
fn is_update_notification(method: &str) -> bool {
    method == "session/update" || method == "_x.ai/session_notification"
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
        "agent_thought_chunk" => {
            flush_tool_group(sink, pending_tools)?;
            if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                if !text.is_empty() {
                    send(
                        sink,
                        AgentEvent::Thought {
                            text: text.to_string(),
                        },
                    )?;
                }
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
        "retry_state" => {
            // Grok streams provider retries here; without this the UI looks
            // hung for minutes while the agent is backing off between attempts.
            if update.get("type").and_then(Value::as_str) == Some("retrying") {
                let attempt = update.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                let max = update.get("max_retries").and_then(Value::as_u64).unwrap_or(0);
                let reason = update
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let reason: String = reason.chars().take(140).collect();
                let text = if max > 0 {
                    format!("Retrying {attempt}/{max}: {reason}")
                } else {
                    format!("Retrying: {reason}")
                };
                send(sink, AgentEvent::Status { text })?;
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
                // Fallback for content shapes the mapper does not know (e.g.
                // `terminal` or future types): keep any text the item carries
                // instead of dropping the update's payload silently.
                _ => {
                    let text = item
                        .pointer("/content/text")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("text").and_then(Value::as_str));
                    if let Some(text) = text.filter(|text| !text.is_empty()) {
                        if !state.output.is_empty() {
                            state.output.push('\n');
                        }
                        state.output.push_str(text);
                    }
                }
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

    // ------------------------------------------------------- client methods

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eb-acp-test-{}-{name}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fs_write_then_read_round_trips() {
        let cwd = tempdir("fs-roundtrip");
        fs_write(
            &cwd,
            acp::WriteTextFileRequest::new(
                acp::SessionId::from("s"),
                "src/out/notes.md",
                "hello\nworld\n",
            ),
        )
        .unwrap();
        // Relative paths land in the session workspace, and missing parents
        // are created — that is what an agent relies on when scaffolding.
        let written = std::fs::read_to_string(cwd.join("src/out/notes.md")).unwrap();
        assert_eq!(written, "hello\nworld\n");

        let content = fs_read(
            &cwd,
            acp::ReadTextFileRequest::new(
                acp::SessionId::from("s"),
                cwd.join("src/out/notes.md"),
            ),
        )
        .unwrap()["content"]
        .clone();
        assert_eq!(content.as_str().unwrap(), "hello\nworld\n");
        std::fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn fs_read_honours_line_and_limit() {
        let cwd = tempdir("fs-window");
        std::fs::write(cwd.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let content = fs_read(
            &cwd,
            acp::ReadTextFileRequest::new(acp::SessionId::from("s"), cwd.join("f.txt"))
                .line(2u32)
                .limit(2u32),
        )
        .unwrap()["content"]
        .clone();
        assert_eq!(content.as_str().unwrap(), "two\nthree");
        std::fs::remove_dir_all(&cwd).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn terminal_create_output_wait_and_release() {
        let cwd = tempdir("terminal");
        let mut terminals = HashMap::new();
        let created = terminal_create(
            &cwd,
            acp::CreateTerminalRequest::new(acp::SessionId::from("s"), "sh").args(vec![
                "-c".into(),
                "echo hello; echo err >&2".into(),
            ]),
            &mut terminals,
        )
        .unwrap();
        let id = created["terminalId"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();

        // wait_for_exit blocks until the reaper stores the status.
        let exit = terminal_wait(wait_request(id), &mut terminals).unwrap();
        assert_eq!(exit["exitCode"].as_i64(), Some(0));

        let output = terminal_output(output_request(id), &mut terminals).unwrap();
        assert_eq!(output["output"].as_str().unwrap(), "hello\nerr\n");
        assert_eq!(output["truncated"].as_bool(), Some(false));
        assert_eq!(output["exitStatus"]["exitCode"].as_i64(), Some(0));

        terminal_release(release_request(id), &mut terminals).unwrap();
        assert!(terminal_output(output_request(id), &mut terminals).is_err());
        std::fs::remove_dir_all(&cwd).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn terminal_kill_stops_a_long_process() {
        let cwd = tempdir("terminal-kill");
        let mut terminals = HashMap::new();
        let created = terminal_create(
            &cwd,
            acp::CreateTerminalRequest::new(acp::SessionId::from("s"), "sh")
                .args(vec!["-c".into(), "sleep 30".into()]),
            &mut terminals,
        )
        .unwrap();
        let id = created["terminalId"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        terminal_kill(kill_request(id), &mut terminals).unwrap();
        let exit = terminal_wait(wait_request(id), &mut terminals).unwrap();
        // A killed process surfaces as a signal, not a clean exit code.
        assert!(exit.get("signal").is_some() || exit["exitCode"].as_i64() != Some(0));
        std::fs::remove_dir_all(&cwd).unwrap();
    }

    fn session_id() -> acp::SessionId {
        acp::SessionId::from("s")
    }

    fn wait_request(id: u64) -> acp::WaitForTerminalExitRequest {
        acp::WaitForTerminalExitRequest::new(session_id(), acp::TerminalId::from(id.to_string()))
    }

    fn output_request(id: u64) -> acp::TerminalOutputRequest {
        acp::TerminalOutputRequest::new(session_id(), acp::TerminalId::from(id.to_string()))
    }

    fn release_request(id: u64) -> acp::ReleaseTerminalRequest {
        acp::ReleaseTerminalRequest::new(session_id(), acp::TerminalId::from(id.to_string()))
    }

    fn kill_request(id: u64) -> acp::KillTerminalRequest {
        acp::KillTerminalRequest::new(session_id(), acp::TerminalId::from(id.to_string()))
    }

    #[test]
    fn terminal_output_trim_keeps_the_tail() {
        let mut buffer = "x".repeat(TERMINAL_OUTPUT_CAP + 5_000);
        assert!(trim_terminal_output(&mut buffer));
        assert_eq!(buffer.len(), TERMINAL_OUTPUT_CAP);
        assert!(buffer.ends_with("xxx"));

        let mut small = String::from("small");
        assert!(!trim_terminal_output(&mut small));
        assert_eq!(small, "small");
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

    fn fixture_config() -> AcpConfig {
        let fixture = format!("{}/tests/fixtures/fake_acp.py", env!("CARGO_MANIFEST_DIR"));
        AcpConfig {
            program: "python3".to_string(),
            args: vec![fixture],
            cwd: std::env::temp_dir(),
            auto_approve: false,
            name: "test".to_string(),
            debug: false,
            // Nothing in the suite may inherit a variable from the shell it
            // happens to run in; `scenario` picks the fixture's behaviour.
            env: Vec::new(),
            auth_method: None,
            tail: TailPolicy {
                first: Duration::from_millis(400),
                quiet: Duration::from_secs(3),
                busy: Duration::from_secs(6),
                max: Duration::from_secs(20),
            },
            log: None,
        }
    }

    fn fixture_scenario(config: &AcpConfig, scenario: &str) -> AcpConfig {
        let mut config = config.clone();
        config.env.push(("FAKE_ACP_SCENARIO".to_string(), scenario.to_string()));
        config
    }

    /// The probe is how a user checks a slot without paying for a turn, so it
    /// must complete the handshake and must not send a prompt.
    #[test]
    fn probe_handshakes_without_prompting() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let probe = AcpAgent::new(fixture_config())
            .probe(&std::env::temp_dir())
            .expect("probe should complete initialize and session/new");
        assert_eq!(probe.agent_name, "Fake ACP");
        assert!(probe.capabilities.is_object(), "got {}", probe.capabilities);

        let missing = AcpConfig {
            program: "/definitely/not/an/agent".to_string(),
            ..fixture_config()
        };
        let error = AcpAgent::new(missing)
            .probe(&std::env::temp_dir())
            .expect_err("a missing executable must not probe clean");
        assert!(
            error.contains("could not launch"),
            "unexpected probe error: {error}"
        );
    }

    fn collect_events(stream: &eb_api::AgentStream) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        loop {
            match stream.recv_timeout(Duration::from_secs(10)) {
                Ok(event) => {
                    let terminal = matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
                    events.push(event);
                    if terminal {
                        return events;
                    }
                }
                Err(error) => panic!("ACP stream ended early: {error:?} (events: {events:?})"),
            }
        }
    }

    /// The declared fs/terminal client capabilities must actually answer: the
    /// fixture agent drives every new method on a real turn and reports what
    /// it saw, so this asserts the round trip through the live JSON-RPC machinery.
    #[test]
    fn client_fs_and_terminal_methods_serve_a_real_turn() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "client_tools"));
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);

        let text = deltas(&events);
        assert!(text.contains("write done"), "fs/write failed: {text:?}");
        assert!(text.contains("read window=two"), "fs/read window wrong: {text:?}");
        assert!(
            text.contains("termout=from-terminal"),
            "terminal output missing: {text:?}"
        );
        assert!(text.contains("termexit=0"), "terminal exit wrong: {text:?}");
        assert!(
            text.contains("released-rejected=True"),
            "a released terminalId must be refused: {text:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
    }

    /// An agent that keeps working after it answered its own turn (Grok's
    /// `deep-research` does this) must not have its follow-up dropped: the turn
    /// stays open, the late text lands in it, and requests in that phase are
    /// still answered.
    #[test]
    fn late_updates_stay_in_the_same_turn() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "tail"));
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);

        let text: String = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Delta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text,
            "Hello from ACP.\n\nresearch: part one. research: part two. research: part three.",
            "continuation deltas were lost: {text:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Status { text } if text == "Agent is still working"
            )),
            "no status for the continuation phase"
        );
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
    }

    /// The agent's text, exactly as the user would read it.
    fn deltas(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Delta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Grok keeps its session `working` for the whole of a background hand-off
    /// and only flips it `idle` when the work is really over, so a turn has to
    /// outlive silence that is longer than the probe window — and then close on
    /// the signal instead of waiting out a timeout after the answer.
    #[test]
    fn a_working_session_outlives_the_probe_and_idle_closes_the_turn() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "tail_busy"));
        let (sink, stream) = AgentSink::channel();
        let started = Instant::now();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);
        let elapsed = started.elapsed();

        let text: String = deltas(&events);
        assert_eq!(
            text,
            "Hello from ACP.\n\nresearch: after the first silence. after the second.",
            "silence longer than the probe window ended a session that said it was working: {text:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
        assert!(
            elapsed < Duration::from_secs(4),
            "the turn did not close on the idle signal (busy window is 6s): {elapsed:?}"
        );
    }

    /// An agent that says `idle` and then keeps streaming is still answering
    /// this turn: the first chunk after the flip revives the continuation window
    /// instead of every later gap being measured against the short probe.
    #[test]
    fn output_that_follows_the_idle_signal_survives_the_probe() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "past_idle"));
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);

        assert_eq!(
            deltas(&events),
            "Hello from ACP.\n\nresearch: after idle, still streaming.",
            "the second burst after `idle` was cut off by the probe window"
        );
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
    }

    /// Work the client already gave up on arrives after the turn closed. It is
    /// nobody's answer, and reading it out as the *next* turn's reply is how a
    /// research run ends up pasted under an unrelated question.
    #[test]
    fn a_late_agent_cannot_answer_the_next_turn() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "backlog"));

        let (first_sink, first_stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("one", first_sink, Arc::new(AllowOnce));
        let first = collect_events(&first_stream);
        assert!(matches!(first.last(), Some(AgentEvent::Finished)));
        assert!(!deltas(&first).contains("LEFTOVER"), "turn one got the late work: {first:?}");

        // The fixture only starts talking again 1.5s after it said idle, so this
        // is when its leftovers pile up in front of a session nobody is reading.
        std::thread::sleep(Duration::from_millis(2_500));

        let (second_sink, second_stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("two", second_sink, Arc::new(AllowOnce));
        let second = collect_events(&second_stream);
        let text = deltas(&second);
        assert!(
            !text.contains("LEFTOVER"),
            "the previous turn's late output was read out as this turn's answer: {text:?}"
        );
        assert!(
            text.contains("Hello from ACP."),
            "the new turn lost its own reply: {text:?}"
        );
    }

    /// The reader keeps the session's `activity` live between turns, and that is
    /// the only evidence a new turn starts with. A session that was still
    /// `working` when its turn gave up announces nothing new — the row only
    /// changes when the state does — so the next turn has to inherit `working`,
    /// or its answer, which lands a beat after the response, is cut off by the
    /// silence probe. A session that said `idle` in between has to inherit
    /// *that*, or every quick answer afterwards rides the busy window.
    #[test]
    fn a_session_still_working_from_an_earlier_turn_keeps_the_next_one_open() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let mut config = fixture_scenario(&fixture_config(), "resume");
        // Turn one never hears `idle` and closes on this window, so keep it short.
        config.tail.busy = Duration::from_millis(2_500);
        let agent = AcpAgent::new(config);

        let (first_sink, first_stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("one", first_sink, Arc::new(AllowOnce));
        let first = collect_events(&first_stream);
        assert!(matches!(first.last(), Some(AgentEvent::Finished)));
        assert_eq!(deltas(&first), "Hello from ACP.");

        let (second_sink, second_stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("two", second_sink, Arc::new(AllowOnce));
        let second = collect_events(&second_stream);
        assert_eq!(
            deltas(&second),
            "Hello from ACP.\n\nresearch: still running.",
            "a turn inherited a session that was already working, said so no more, and was ended at the probe"
        );
        assert!(matches!(second.last(), Some(AgentEvent::Finished)));

        // Turn two ended on `idle`, so turn three is an ordinary short answer.
        let (third_sink, third_stream) = AgentSink::channel();
        let started = Instant::now();
        let _handle = agent.start_with_permissions("three", third_sink, Arc::new(AllowOnce));
        let third = collect_events(&third_stream);
        let elapsed = started.elapsed();
        assert_eq!(deltas(&third), "Hello from ACP.");
        assert!(
            elapsed < Duration::from_millis(1_500),
            "a session that said `idle` was still held open on the busy window: {elapsed:?}"
        );
    }

    /// A flip that arrives between turns is queued, not lost: the work the client
    /// gave up on finished, and the next prompt has to start from that `idle`. An
    /// agent that answers in half a second would otherwise sit in the busy window
    /// with its reply already on screen.
    #[test]
    fn an_idle_that_arrives_between_turns_starts_the_next_one() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let mut config = fixture_scenario(&fixture_config(), "idle_between");
        // Turn one hears no `idle` before this window runs out, then the flip lands.
        config.tail.busy = Duration::from_millis(2_500);
        let agent = AcpAgent::new(config);

        let (first_sink, first_stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("one", first_sink, Arc::new(AllowOnce));
        let first = collect_events(&first_stream);
        assert!(matches!(first.last(), Some(AgentEvent::Finished)));

        // The fixture flips `idle` 4s after answering, a second and a half after
        // this turn ran out its busy window, so the queue holds it when the next
        // prompt goes out.
        std::thread::sleep(Duration::from_millis(2_500));

        let (second_sink, second_stream) = AgentSink::channel();
        let started = Instant::now();
        let _handle = agent.start_with_permissions("two", second_sink, Arc::new(AllowOnce));
        let second = collect_events(&second_stream);
        let elapsed = started.elapsed();
        assert_eq!(deltas(&second), "Hello from ACP.");
        assert!(
            elapsed < Duration::from_millis(1_500),
            "the queued `idle` was dropped with the rest of the backlog: {elapsed:?}"
        );
    }

    /// The tail policy is argued from transcripts of real agents, so
    /// `EVOBUDDY_ACP_LOG` has to actually produce one — every message the agent
    /// sends, raw, timestamped, in arrival order, straight from the reader thread.
    #[test]
    fn the_transcript_log_records_every_raw_message() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let path = std::env::temp_dir().join(format!(
            "evobuddy-acp-log-{}-{}.jsonl",
            std::process::id(),
            "transcript"
        ));
        let _ = std::fs::remove_file(&path);

        let mut config = fixture_scenario(&fixture_config(), "tail");
        config.log = Some(path.clone());
        let agent = AcpAgent::new(config);
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));

        let transcript = std::fs::read_to_string(&path).expect("the reader thread writes the log");
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = transcript.lines().collect();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\"stopReason\":\"end_turn\"")),
            "the log did not record the prompt response: {lines:?}"
        );
        // Everything the turn needs to be argued about is here, in arrival order:
        // the response, then the continuation it was followed by.
        let position = |needle: &str| transcript.find(needle).unwrap_or(usize::MAX);
        assert!(
            position("\"stopReason\":\"end_turn\"") < position("part one")
                && position("part one") < position("part two")
                && position("part two") < position("part three"),
            "the tail was not recorded after the response: {lines:?}"
        );
        assert!(
            lines.iter().all(|line| {
                let mut parts = line.split('\t');
                let seconds = parts.next().unwrap_or("").parse::<f64>();
                seconds.is_ok() && parts.next().is_some() && parts.next().is_none()
            }),
            "each line must be one tab-separated <unix seconds>\\t<raw message>: {lines:?}"
        );
        assert!(
            lines.len() > 3,
            "only {} message(s) recorded: {lines:?}",
            lines.len()
        );
    }

    /// When the background work dies *after* the response, the private
    /// `turn_completed` notice is the only explanation the agent leaves behind;
    /// the turn must fail with its reason instead of ending empty and "successful".
    #[test]
    fn a_failed_continuation_ends_the_turn_with_the_agent_reason() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "tail_error"));
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);

        match events.last() {
            Some(AgentEvent::Failed { message }) => {
                assert!(
                    message.contains("Connection reset by peer"),
                    "the agent's reason was lost: {message}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Grok reports provider backoff as `retry_state`; a turn that is silently
    /// retrying for minutes has to say so in the status line.
    #[test]
    fn provider_retries_reach_the_status_line() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let agent = AcpAgent::new(fixture_scenario(&fixture_config(), "retry"));
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start_with_permissions("hello", sink, Arc::new(AllowOnce));
        let events = collect_events(&stream);

        let status: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Status { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            status
                .iter()
                .any(|text| text.starts_with("Retrying 1/15") && text.contains("Connection reset")),
            "retry_state in Grok's private envelope never surfaced: {status:?}"
        );
        assert!(
            status.iter().any(|text| text.starts_with("Retrying 2/15")),
            "retry_state in the spec envelope never surfaced: {status:?}"
        );
    }

    #[test]
    fn fake_acp_maps_text_usage_exec_and_diff() {
        if !python_available() {
            eprintln!("skipping ACP integration test: python3 not available");
            return;
        }
        let config = fixture_config();
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
            AgentEvent::Thought { text } if text == "Thinking: "
        )));
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
        let config = fixture_scenario(&fixture_config(), "cancel");
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
    #[test]
    fn connector_files_become_acp_mcp_servers() {
        let dir = std::env::temp_dir().join(format!("eb-connectors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("stdio.json"),
            r#"{"name":"files","command":"npx","args":["-y","mcp-files"],"env":{"TOKEN":"x"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("remote.json"),
            r#"{"mcpServers":{"search":{"url":"https://example.test/mcp"}}}"#,
        )
        .unwrap();
        let servers = read_mcp_servers(&dir);
        assert!(servers.iter().any(|server| server["name"] == "files"
            && server["command"] == "npx"
            && server["env"]["TOKEN"] == "x"));
        assert!(servers.iter().any(|server| server["name"] == "search"
            && server["url"] == "https://example.test/mcp"));
        let _ = std::fs::remove_dir_all(&dir);
    }

}
