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
    PermissionOption, PermissionRequest, PermissionResponder,
};
use eb_api::model::Block;

/// Tool rounds per turn, so a confused model cannot loop forever. When the
/// budget runs out the turn is not failed outright: one last request goes
/// out with the tools withheld, so the model must answer in prose.
const MAX_TOOL_ROUNDS: usize = 32;
/// Command output kept per tool result, tail-trimmed.
const TOOL_OUTPUT_CAP: usize = 8_000;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CustomConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Context window in tokens. Optional: when empty the window is probed
    /// from `{base}/models` and, failing that, learned from a
    /// context-length error. Compaction kicks in at 85% of this budget.
    pub context_length: Option<u64>,
}

/// Compaction kicks in when the estimated prompt reaches this share of the
/// context window; the rest is headroom for the answer itself.
const COMPACTION_THRESHOLD: f64 = 0.85;
/// Tool results kept verbatim during compaction; older ones are blanked.
const TOOL_KEEP_RECENT: usize = 3;
/// Replacement left behind for a blanked tool result.
const OMITTED_TOOL: &str = "[earlier tool result omitted to fit the context window]";
/// Cross-turn transcript cap when the window is unknown (previous behavior).
const DEFAULT_TRANSCRIPT_CAP: usize = 24_000;
/// How many characters one token is worth, before usage feedback calibrates it.
const DEFAULT_CHARS_PER_TOKEN: f64 = 4.0;

/// Shared across turns (and across task instances) so the window has to be
/// detected only once per endpoint. `chars_per_token` is the observed
/// serialized-bytes-per-prompt-token ratio from `usage.prompt_tokens`.
#[derive(Debug)]
struct WindowCache {
    limit_tokens: Option<u64>,
    chars_per_token: f64,
}

static WINDOW: std::sync::OnceLock<Mutex<WindowCache>> = std::sync::OnceLock::new();

fn window_cache() -> &'static Mutex<WindowCache> {
    WINDOW.get_or_init(|| {
        Mutex::new(WindowCache {
            limit_tokens: None,
            chars_per_token: DEFAULT_CHARS_PER_TOKEN,
        })
    })
}

/// Remember a detected context window, keeping the smallest known value:
/// a hard error is more authoritative than a generous `/models` claim.
fn remember_window_limit(tokens: u64) {
    if !(256..=100_000_000).contains(&tokens) {
        return;
    }
    if let Ok(mut cache) = window_cache().lock() {
        cache.limit_tokens = Some(match cache.limit_tokens {
            Some(known) => known.min(tokens),
            None => tokens,
        });
    }
}

fn window_limit() -> Option<u64> {
    window_cache().lock().ok().and_then(|c| c.limit_tokens)
}

/// Calibrate the bytes-per-token ratio from a real `usage.prompt_tokens`
/// reading, so later estimates track the actual tokenizer instead of the
/// ASCII/CJK heuristic.
fn calibrate_chars_per_token(serialized_bytes: usize, prompt_tokens: u64) {
    if prompt_tokens == 0 || serialized_bytes == 0 {
        return;
    }
    let ratio = serialized_bytes as f64 / prompt_tokens as f64;
    if !(1.0..=16.0).contains(&ratio) {
        return;
    }
    if let Ok(mut cache) = window_cache().lock() {
        // Blend toward the new observation; a single reading must not swing it.
        cache.chars_per_token = (cache.chars_per_token * 0.5 + ratio * 0.5).clamp(1.5, 12.0);
    }
}

fn calibrated_chars_per_token() -> f64 {
    window_cache()
        .lock()
        .map(|c| c.chars_per_token)
        .unwrap_or(DEFAULT_CHARS_PER_TOKEN)
}

/// Character budget for the host-side cross-turn transcript: 85% of the
/// detected window converted through the calibrated ratio, falling back to
/// the historical 24 000 characters when nothing is known.
pub fn transcript_char_cap() -> usize {
    match window_limit() {
        Some(tokens) => ((tokens as f64 * COMPACTION_THRESHOLD * calibrated_chars_per_token())
            as usize)
            .clamp(2_000, 4_000_000),
        None => DEFAULT_TRANSCRIPT_CAP,
    }
}

/// Pull the context length out of a context-overflow error message. Covers
/// the OpenAI phrasing ("maximum context length is 8192 tokens"), vLLM
/// ("max context length is ... tokens") and the Chinese variants
/// ("上下文长度为 8192"/"上下文窗口...8192").
pub fn parse_context_limit(message: &str) -> Option<u64> {
    let lower = message.to_lowercase();
    for keyword in ["context length", "context window", "上下文长度", "上下文窗口"] {
        let Some(position) = lower.find(keyword) else {
            continue;
        };
        // ASCII keywords and CJK keywords survive `to_lowercase` byte-for-byte,
        // so the offset is valid in the original string too.
        let tail = &message[position + keyword.len()..];
        let digits: String = tail
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(tokens) = digits.parse::<u64>() {
            if (256..=100_000_000).contains(&tokens) {
                return Some(tokens);
            }
        }
    }
    None
}

/// Where an overflow can surface: the SSE error payload, curl's stderr
/// (some gateways put the HTTP 400 body there), or a non-streamed JSON
/// error body. `raw` is only scanned when the cheap sources came up empty.
fn context_limit_from(outcome: &RoundOutcome) -> Option<u64> {
    if !outcome.stream_error.is_empty() {
        if let Some(limit) = parse_context_limit(&outcome.stream_error) {
            return Some(limit);
        }
    }
    if !outcome.errors.is_empty() {
        if let Some(limit) = parse_context_limit(&outcome.errors) {
            return Some(limit);
        }
    }
    parse_context_limit(&outcome.raw)
}

/// Serialized byte size of a message list — the input to both estimators.
fn messages_bytes(messages: &[serde_json::Value]) -> usize {
    serde_json::to_string(&messages)
        .map(|text| text.len())
        .unwrap_or(0)
}

/// Rough token estimate for a message list. Once real `usage` feedback has
/// calibrated the ratio, the serialized byte size divided by it is the best
/// guess; before that, a per-byte heuristic (ASCII ~4 bytes/token, UTF-8
/// CJK ~3 bytes/token) errs on the compacting-early side.
fn estimate_tokens_messages(messages: &[serde_json::Value]) -> f64 {
    let Ok(text) = serde_json::to_string(&messages) else {
        return f64::MAX;
    };
    let ratio = calibrated_chars_per_token();
    if (ratio - DEFAULT_CHARS_PER_TOKEN).abs() > f64::EPSILON {
        return text.len() as f64 / ratio;
    }
    let ascii = text.bytes().filter(|byte| *byte < 128).count();
    let non_ascii = text.len() - ascii;
    ascii as f64 * 0.25 + non_ascii as f64 * 0.33
}

/// Shrink a turn's message list toward `budget_tokens`. Two mechanical
/// passes, in order of information loss:
///
/// 1. blank the bodies of every tool result except the most recent
///    [`TOOL_KEEP_RECENT`] — tool output is the bulk of a long turn;
/// 2. drop the oldest complete rounds, always keeping the first message
///    (the original request) and a self-contained tail (a round never
///    loses its assistant `tool_calls` message).
///
/// Returns whether anything changed. Purely mechanical: no extra model
/// request, so it cannot recurse into the same overflow.
fn compact_messages(messages: &mut Vec<serde_json::Value>, budget_tokens: f64) -> bool {
    if messages.is_empty() || estimate_tokens_messages(messages) <= budget_tokens {
        return false;
    }
    let mut changed = false;

    // Pass 1: blank old tool bodies.
    let tool_indices: Vec<usize> = (0..messages.len())
        .filter(|&index| messages[index]["role"] == "tool")
        .collect();
    let keep_from = tool_indices.len().saturating_sub(TOOL_KEEP_RECENT);
    for &index in &tool_indices[..keep_from] {
        if messages[index]["content"].as_str() != Some(OMITTED_TOOL) {
            messages[index]["content"] = serde_json::json!(OMITTED_TOOL);
            changed = true;
        }
    }

    // Pass 2: drop whole rounds from the front until the estimate fits.
    while estimate_tokens_messages(messages) > budget_tokens && messages.len() > 2 {
        // The last self-contained round start: a user message or an
        // assistant message carrying tool_calls. Everything between it and
        // the front can go without orphaning a tool result.
        let Some(start) = (1..messages.len()).rev().find(|&index| {
            let message = &messages[index];
            message["role"] == "user"
                || (message["role"] == "assistant" && message["tool_calls"].is_array())
        }) else {
            break;
        };
        if start <= 1 {
            break;
        }
        messages.drain(1..start);
        changed = true;
    }
    changed
}

/// One-time `{base}/models` probe. vLLM exposes `max_model_len`, LM Studio
/// and some gateways `context_length` or `context_window`; servers that
/// return neither (OpenAI itself, bigmodel.cn) yield `None` and the
/// error-driven detection covers them later.
fn probe_models_limit(base_url: &str, key: &str, model: &str) -> Option<u64> {
    let base = base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    let mut command = Command::new("curl");
    command
        .args(["-sS", "--max-time", "8", &format!("{base}/models")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !key.trim().is_empty() {
        command.arg("-H").arg(format!("Authorization: Bearer {key}"));
    }
    let Ok(output) = command.output() else {
        return None;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return None;
    };
    let entries = value["data"].as_array()?;
    let matching = entries
        .iter()
        .find(|entry| entry["id"].as_str() == Some(model))
        .or_else(|| entries.first())?;
    ["context_length", "max_model_len", "context_window"]
        .iter()
        .find_map(|field| matching[field].as_u64())
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
}

impl AnswerSource for CustomModel {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        self.start_in(prompt, sink, Arc::new(eb_api::DenyAll), None)
    }

    fn start_in(
        &self,
        prompt: &str,
        sink: AgentSink,
        _permissions: Arc<dyn eb_api::PermissionPrompter>,
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
        let configured_limit = self.config.context_length;
        let cancelled = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let child_for_handle = child.clone();
        let cancelled_for_worker = cancelled.clone();
        std::thread::Builder::new()
            .name("evobuddy-custom-model".into())
            .spawn(move || {
                // A panic in here used to kill the thread silently: the
                // stream channel closed and the user only saw the generic
                // "agent stream closed". Catch it and report the truth.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // The turn-scoped consent memory, plus the inline ask: the
                // request renders as a card in the transcript and the answer
                // arrives through the responder.
                let mut consent = crate::tools::ToolConsent::default();
                let mut jobs = crate::tools::exec::JobTable::default();
                let mut ask = |_class: &str, title: &str| -> PermissionDecision {
                    // The headless verification path answers before the card:
                    // EB_PERMISSION_AUTO=allow|reject short-circuits it.
                    match std::env::var("EB_PERMISSION_AUTO").as_deref() {
                        Ok("allow") => return PermissionDecision::Selected("allow_once".into()),
                        Ok("reject") => return PermissionDecision::Cancelled,
                        _ => {}
                    }
                    let (reply, receiver) = std::sync::mpsc::channel();
                    let request = PermissionRequest {
                        title: title.to_string(),
                        options: vec![
                            PermissionOption {
                                id: "allow_once".into(),
                                label: "Allow".into(),
                                kind: "allow_once".into(),
                            },
                            PermissionOption {
                                id: "allow_always".into(),
                                label: "Always allow".into(),
                                kind: "allow_always".into(),
                            },
                            PermissionOption {
                                id: "reject_once".into(),
                                label: "Reject".into(),
                                kind: "reject_once".into(),
                            },
                        ],
                    };
                    let _ = sink.send(AgentEvent::PermissionRequested {
                        request,
                        responder: PermissionResponder::new(reply),
                    });
                    loop {
                        if cancelled_for_worker.load(Ordering::SeqCst) {
                            return PermissionDecision::Cancelled;
                        }
                        match receiver
                            .recv_timeout(std::time::Duration::from_millis(200))
                        {
                            Ok(decision) => return decision,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(_) => return PermissionDecision::Cancelled,
                        }
                    }
                };
                // One request/execute round. `tools` is `None` on the
                // wrap-up call after the budget runs out, which takes the
                // toolset away so the model has to answer in prose.
                let request = |messages: &[serde_json::Value],
                               tools: Option<serde_json::Value>|
                 -> RoundOutcome {
                    let mut body = serde_json::json!({
                        "model": model,
                        "messages": messages,
                        "stream": true,
                    });
                    if let Some(tools) = tools {
                        body["tools"] = tools;
                    } else {
                        body["tool_choice"] = serde_json::json!("none");
                    }
                    request_round(&body, &endpoint, &key, &child, &cancelled_for_worker, &sink)
                };
                // Establish the context budget once per turn: explicit config
                // wins, then whatever earlier turns learned, then a one-time
                // `/models` probe. Servers that answer none of these stay
                // unbounded until an overflow error teaches us the limit.
                let mut budget_tokens = configured_limit
                    .or_else(window_limit)
                    .or_else(|| {
                        probe_models_limit(&base_url, &key, &model)
                            .inspect(|tokens| remember_window_limit(*tokens))
                    })
                    .map(|tokens| tokens as f64 * COMPACTION_THRESHOLD);
                let mut healed = false;
                for _round in 0..MAX_TOOL_ROUNDS {
                    if let Some(budget) = budget_tokens {
                        compact_messages(&mut messages, budget);
                    }
                    let mut outcome = request(&messages, Some(crate::tools::tool_definitions()));
                    if !outcome.started {
                        // curl could not spawn; the failure is already out.
                        return;
                    }
                    // Self-heal a context-length overflow: learn the window
                    // from the error text, compact, and retry this round once.
                    if !healed {
                        if let Some(limit) = context_limit_from(&outcome) {
                            remember_window_limit(limit);
                            let compact_budget = limit as f64 * 0.70;
                            compact_messages(&mut messages, compact_budget);
                            budget_tokens = Some(limit as f64 * COMPACTION_THRESHOLD);
                            healed = true;
                            outcome = request(&messages, Some(crate::tools::tool_definitions()));
                            if !outcome.started {
                                return;
                            }
                        }
                    }
                    if let Some(tokens) = outcome.prompt_tokens {
                        calibrate_chars_per_token(messages_bytes(&messages), tokens);
                    }
                    if cancelled_for_worker.load(Ordering::SeqCst) {
                        let _ = sink.send(AgentEvent::Failed { message: "Cancelled".into() });
                        return;
                    }

                    let calls = outcome.calls;
                    if calls.is_empty() {
                        // No tools requested: the streamed content (or the
                        // plain-JSON fallback) is the answer.
                        if !outcome.stream_error.is_empty() {
                            let _ = sink.send(AgentEvent::Failed {
                                message: hint_for(&outcome.stream_error, &base_url),
                            });
                            return;
                        }
                        if !outcome.streamed.is_empty() {
                            let _ = sink.send(AgentEvent::Finished);
                            return;
                        }
                        // A server that ignored `stream` answered with one
                        // plain JSON body: parse it as the answer.
                        let failure = match parse_response(&outcome.raw) {
                            Ok(text) => {
                                let _ = sink.send(AgentEvent::Block {
                                    block: Block::Text(text),
                                });
                                let _ = sink.send(AgentEvent::Finished);
                                return;
                            }
                            Err(error) => error,
                        };
                        let detail = outcome
                            .errors
                            .lines()
                            .last()
                            .unwrap_or("")
                            .trim()
                            .to_string();
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

                    // Tool round: append the assistant's tool_calls (keeping
                    // any prose it streamed, so it does not restate it next
                    // round), execute each, and feed the results back for
                    // the next round.
                    let mut assistant = serde_json::json!({ "role": "assistant" });
                    if !outcome.streamed.trim().is_empty() {
                        assistant["content"] = serde_json::json!(outcome.streamed);
                    }
                    assistant["tool_calls"] = serde_json::json!(calls
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
                        .collect::<Vec<_>>());
                    messages.push(assistant);
                    for call in &calls {
                        let result =
                            crate::tools::execute_tool(call, &root, &mut consent, &mut ask, &sink, &mut jobs);
                        messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": result,
                        }));
                    }
                }

                // The budget ran out. Rather than failing the whole turn,
                // take the tools away and ask for a wrap-up: the model must
                // summarize what it did and what is left, instead of
                // reaching for another tool call.
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": "The tool round budget for this turn is exhausted. \
Do not call any more tools. Give your final answer now, summarizing what \
you did and what remains undone.",
                }));
                if let Some(budget) = budget_tokens {
                    compact_messages(&mut messages, budget);
                }
                let outcome = request(&messages, None);
                if !outcome.started {
                    return;
                }
                if !outcome.streamed.trim().is_empty() {
                    let _ = sink.send(AgentEvent::Finished);
                    return;
                }
                // Nothing streamed: either a non-streaming reply or a server
                // that still returned tool calls despite `tool_choice`. Try
                // the plain-JSON parse, then fall back to the budget error.
                if let Ok(text) = parse_response(&outcome.raw) {
                    let _ = sink.send(AgentEvent::Block {
                        block: Block::Text(text),
                    });
                    let _ = sink.send(AgentEvent::Finished);
                    return;
                }
                let _ = sink.send(AgentEvent::Failed {
                    message: format!(
                        "tool loop exceeded {MAX_TOOL_ROUNDS} rounds without a final answer"
                    ),
                });
                }));
                if outcome.is_err() {
                    // The default hook already printed the panic to stderr.
                    let _ = sink.send(AgentEvent::Failed {
                        message: "model worker panicked (details on stderr; run with EB_DEBUG=1 for event-level logs)".into(),
                    });
                }
            })
            .ok();
        Box::new(CustomHandle {
            cancelled,
            child: child_for_handle,
        })
    }
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

/// One model request. Streams `Delta`/`Thought` events through `sink`,
/// accumulates the answer text, raw body, stderr, stream error and tool-call
/// fragments. `started` is false when curl could not spawn; the failure has
/// already been reported to the sink in that case.
struct RoundOutcome {
    started: bool,
    streamed: String,
    raw: String,
    errors: String,
    stream_error: String,
    calls: Vec<crate::tools::ToolCall>,
    /// `usage.prompt_tokens` from the server, when it reports one. Each
    /// reading recalibrates the token estimator for the compaction budget.
    prompt_tokens: Option<u64>,
}

fn request_round(
    body: &serde_json::Value,
    endpoint: &str,
    key: &str,
    child: &Arc<Mutex<Option<Child>>>,
    cancelled: &AtomicBool,
    sink: &AgentSink,
) -> RoundOutcome {
    let mut outcome = RoundOutcome {
        started: false,
        streamed: String::new(),
        raw: String::new(),
        errors: String::new(),
        stream_error: String::new(),
        calls: Vec::new(),
        prompt_tokens: None,
    };
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
            endpoint,
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
        return outcome;
    };
    // Take the stdin pipe so it drops right after the write:
    // curl reads the body to EOF and must see the pipe close.
    if let Some(mut stdin) = process.stdin.take() {
        let _ = stdin.write_all(body.to_string().as_bytes());
    }
    if let Ok(mut slot) = child.lock() {
        *slot = Some(process);
    }
    outcome.started = true;

    // Stream the SSE lines: content deltas land as Delta, reasoning chunks
    // as Thought, and tool-call fragments accumulate for execution once the
    // stream ends.
    let stdout = child
        .lock()
        .ok()
        .and_then(|mut slot| slot.as_mut().and_then(|p| p.stdout.take()));
    let mut calls: BTreeMap<usize, crate::tools::ToolCall> = BTreeMap::new();
    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            outcome.raw.push_str(&line);
            outcome.raw.push('\n');
            let payload = match line.strip_prefix("data:") {
                Some(payload) => payload.trim(),
                None => continue,
            };
            if payload == "[DONE]" {
                break;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };
            if let Some(message) = value["error"]["message"].as_str() {
                outcome.stream_error = message.to_string();
                break;
            }
            if let Some(tokens) = value["usage"]["prompt_tokens"].as_u64() {
                outcome.prompt_tokens = Some(tokens);
            }
            let choice = &value["choices"][0];
            let delta = &choice["delta"];
            if let Some(piece) = delta["content"].as_str() {
                if !piece.is_empty() {
                    outcome.streamed.push_str(piece);
                    let _ = sink.send(AgentEvent::Delta {
                        text: piece.to_string(),
                    });
                }
            }
            for (key, event) in [("reasoning_content", "thought"), ("reasoning", "thought")] {
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
                    if let Some(name) = fragment["function"]["name"].as_str() {
                        entry.name = name.to_string();
                    }
                    if let Some(arguments) = fragment["function"]["arguments"].as_str() {
                        entry.arguments.push_str(arguments);
                    }
                }
            }
        }
    }

    // Drain stderr and wait for the exit so the handle's kill stays
    // meaningful to the very end.
    if let Ok(mut slot) = child.lock() {
        if let Some(process) = slot.as_mut() {
            if let Some(mut stderr) = process.stderr.take() {
                let _ = stderr.read_to_string(&mut outcome.errors);
            }
            let _ = process.wait();
        }
    }
    outcome.calls = calls.into_values().collect();
    outcome
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
    use crate::tools::exec::JobTable;
    use crate::tools::{ToolCall, ToolConsent};
    use std::io::BufRead;

    /// Legacy-signature shim over the tools-module dispatcher, so the test
    /// call sites below stay focused on behavior, not plumbing.
    fn execute_tool(
        call: &ToolCall,
        root: &std::path::Path,
        consent: &mut ToolConsent,
        ask: &mut dyn FnMut(&str, &str) -> eb_api::PermissionDecision,
        sink: &AgentSink,
    ) -> String {
        let mut jobs = JobTable::default();
        crate::tools::execute_tool(call, root, consent, ask, sink, &mut jobs)
    }

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
            ..Default::default()
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        let handle = model.start("what is 2 + 2", sink);
        let mut text = String::new();
        let mut deltas = 0usize;
        let mut thoughts = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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
        let _guard = mode_lock();
        // Entry self-cleanup: a panicked sibling can leave the global mode or
        // env corrupted; these tests each start from a known state.
        crate::router::set_permission_mode(0);
        std::env::remove_var("EB_PERMISSION_AUTO");
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
            ..Default::default()
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        std::env::set_var("EB_PERMISSION_AUTO", "allow");
        let handle = model.start_in(
            "write it",
            sink,
            Arc::new(eb_api::DenyAll),
            Some(workspace.clone()),
        );
        let mut text = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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
        std::env::remove_var("EB_PERMISSION_AUTO");
        assert!(finished, "the tool round never finished");
        assert_eq!(text, "tool result seen");
        let written =
            std::fs::read_to_string(workspace.join("tool-test/out.txt")).unwrap();
        assert_eq!(written, "hello from the model");
        std::fs::remove_dir_all(&workspace).unwrap();
    }

    /// The full multi-tool script from the mock: search_content finds the
    /// seeded marker, replace_in_file edits the line, todo_write publishes
    /// the plan, and the model answers in prose. This exercises the round
    /// loop feeding results back across four requests — the plumbing the
    /// single-tool test cannot see.
    #[test]
    fn multi_tool_turn_searches_edits_plans_and_answers() {
        let _guard = mode_lock();
        crate::router::set_permission_mode(0);
        std::env::remove_var("EB_PERMISSION_AUTO");
        if !python_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        let workspace = std::env::temp_dir().join(format!(
            "eb-custom-multi-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("todo.txt"), "REPLACE_MARKER original\n").unwrap();

        let mut server = std::process::Command::new("python3")
            .args(["tests/fixtures/openai_mock.py"])
            .env("MOCK_MODE", "multi")
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
            ..Default::default()
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        std::env::set_var("EB_PERMISSION_AUTO", "allow");
        let handle = model.start_in(
            "fix the marker",
            sink,
            Arc::new(eb_api::DenyAll),
            Some(workspace.clone()),
        );

        let mut text = String::new();
        let mut saw_search = false;
        let mut saw_edit = false;
        let mut saw_plan = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
        let finished = loop {
            if std::time::Instant::now() > deadline {
                break false;
            }
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(eb_api::AgentEvent::Delta { text: value }) => text.push_str(&value),
                Ok(eb_api::AgentEvent::Block { block }) => match block {
                    Block::Tool { status, primary, .. } => {
                        saw_search |= status == "Ran" && primary.starts_with("Search ");
                        saw_edit |= status == "Ran" && primary.starts_with("Edit ");
                    }
                    Block::Plan { entries } => {
                        saw_plan = entries.len() == 3
                            && entries.iter().all(|entry| entry.status == "completed");
                    }
                    _ => {}
                },
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
        std::env::remove_var("EB_PERMISSION_AUTO");
        assert!(finished, "the multi-tool turn never finished");
        assert!(saw_search, "no Search tool row on the transcript");
        assert!(saw_edit, "no Edit tool row on the transcript");
        assert!(saw_plan, "no Plan card on the transcript");
        assert_eq!(text, "multi-tool turn complete");
        let edited =
            std::fs::read_to_string(workspace.join("todo.txt")).unwrap();
        assert_eq!(edited, "REPLACE_MARKER edited\n");
        std::fs::remove_dir_all(&workspace).unwrap();
    }

    #[test]
    fn the_permission_mode_answers_without_prompting() {
        let _guard = mode_lock();
        // Entry self-cleanup: a panicked sibling can leave the global mode or
        // env corrupted; these tests each start from a known state.
        crate::router::set_permission_mode(0);
        std::env::remove_var("EB_PERMISSION_AUTO");
        crate::router::set_permission_mode(1); // Auto-approve
        let (sink, _stream) = eb_api::AgentSink::channel();
        let prompts = std::cell::Cell::new(0usize);
        let mut consent = ToolConsent::default();
        let mut ask = |_class: &str, _title: &str| -> PermissionDecision {
            prompts.set(prompts.get() + 1);
            PermissionDecision::Cancelled
        };
        let call = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: json_args(r#"{"command": "echo hi"}"#),
        };
        let result = execute_tool(
            &call,
            std::path::Path::new("/tmp"),
            &mut consent,
            &mut ask,
            &sink,
        );
        assert_eq!(result, "exit 0
hi");
        assert_eq!(prompts.get(), 0, "auto-approve must not prompt");

        crate::router::set_permission_mode(2); // Read-only
        let result = execute_tool(
            &call,
            std::path::Path::new("/tmp"),
            &mut consent,
            &mut ask,
            &sink,
        );
        assert_eq!(result, "user rejected this command");
        crate::router::set_permission_mode(0);
    }

    #[test]
    fn denials_are_reported_as_tool_results() {
        let _guard = mode_lock();
        // Entry self-cleanup: a panicked sibling can leave the global mode or
        // env corrupted; these tests each start from a known state.
        crate::router::set_permission_mode(0);
        std::env::remove_var("EB_PERMISSION_AUTO");
        let (sink, _stream) = eb_api::AgentSink::channel();
        let mut denials = 0usize;
        let mut consent = ToolConsent::default();
        let mut ask = |_class: &str, title: &str| -> PermissionDecision {
            denials += 1;
            let _ = title;
            PermissionDecision::Cancelled
        };
        let call = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: r#"{"command": "echo hi"}"#.into(),
        };
        let result = execute_tool(
            &call,
            std::path::Path::new("/tmp"),
            &mut consent,
            &mut ask,
            &sink,
        );
        assert_eq!(result, "user rejected this command");
        assert_eq!(denials, 1, "the rejection went through the ask");
    }

    #[test]
    fn read_file_lists_and_errors() {
        let (sink, _stream) = eb_api::AgentSink::channel();
        // Reads never prompt.
        let mut consent = ToolConsent::default();
        let mut ask = |_class: &str, _title: &str| -> PermissionDecision {
            panic!("reads must not ask")
        };
        let dir = std::env::temp_dir().join(format!("eb-custom-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "abc").unwrap();

        let read = ToolCall {
            id: "t".into(),
            name: "read_file".into(),
            arguments: json_args(r#"{"path": "a.txt"}"#),
        };
        let mut consent = ToolConsent::default();
        // The new read_file prefixes a line-count header.
        let read_result = execute_tool(&read, &dir, &mut consent, &mut ask, &sink);
        assert!(
            read_result.starts_with("(1 lines)") && read_result.ends_with("abc"),
            "{read_result}"
        );
        let list = ToolCall {
            id: "t".into(),
            name: "list_dir".into(),
            arguments: json_args(r#"{"path": "."}"#),
        };
        let listing = execute_tool(&list, &dir, &mut consent, &mut ask, &sink);
        assert!(listing.contains("a.txt") && listing.contains("sub/"));
        let missing = ToolCall {
            id: "t".into(),
            name: "read_file".into(),
            arguments: json_args(r#"{"path": "nope.txt"}"#),
        };
        assert!(execute_tool(&missing, &dir, &mut consent, &mut ask, &sink)
            .starts_with("read failed:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn json_args(raw: &str) -> String {
        raw.to_string()
    }

    #[test]
    fn always_allow_remembers_the_class_for_the_turn() {
        let _guard = mode_lock();
        // Entry self-cleanup: a panicked sibling can leave the global mode or
        // env corrupted; these tests each start from a known state.
        crate::router::set_permission_mode(0);
        std::env::remove_var("EB_PERMISSION_AUTO");
        std::env::remove_var("EB_PERMISSION_AUTO");
        let (sink, _stream) = eb_api::AgentSink::channel();
        use std::cell::RefCell as RcCell;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let prompts = std::rc::Rc::new(AtomicUsize::new(0));
        let mut consent = ToolConsent::default();
        let prompts_for_ask = prompts.clone();
        // Always answers "Always allow"; consent remembering is the code
        // under test, so the ask itself is class-blind.
        let mut ask = move |_class: &str, _title: &str| -> PermissionDecision {
            prompts_for_ask.fetch_add(1, AtomicOrdering::SeqCst);
            PermissionDecision::Selected("allow_always".into())
        };

        let dir = std::env::temp_dir().join(format!("eb-consent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: json_args(r#"{"command": "echo one"}"#),
        };
        let second = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            arguments: json_args(r#"{"command": "echo two"}"#),
        };
        execute_tool(&first, &dir, &mut consent, &mut ask, &sink);
        assert_eq!(prompts.load(AtomicOrdering::SeqCst), 1, "the first call must prompt");
        execute_tool(&second, &dir, &mut consent, &mut ask, &sink);
        assert_eq!(
            prompts.load(AtomicOrdering::SeqCst),
            1,
            "Always allow must cover the rest of the turn without re-prompting"
        );
        // The other class is still prompt-worthy.
        let write = ToolCall {
            id: "t".into(),
            name: "write_file".into(),
            arguments: json_args(r#"{"path": "x.txt", "content": "hi"}"#),
        };
        execute_tool(&write, &dir, &mut consent, &mut ask, &sink);
        assert_eq!(prompts.load(AtomicOrdering::SeqCst), 2, "a different class prompts again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_limit_errors_are_parsed_across_providers() {
        // OpenAI's classic phrasing.
        assert_eq!(
            parse_context_limit(
                "This model's maximum context length is 8192 tokens. However, your messages resulted in 9000 tokens."
            ),
            Some(8192)
        );
        // vLLM.
        assert_eq!(
            parse_context_limit("This model's max context length is 32768 tokens"),
            Some(32768)
        );
        // Chinese gateways.
        assert_eq!(parse_context_limit("错误：上下文长度为 128000，超出限制"), Some(128000));
        assert_eq!(parse_context_limit("上下文窗口 8192 已满"), Some(8192));
        // Unrelated errors must not produce a number.
        assert_eq!(parse_context_limit("insufficient quota"), None);
        assert_eq!(parse_context_limit("mock: bad auth"), None);
    }

    #[test]
    fn compaction_blanks_old_tool_results_then_drops_old_rounds() {
        let big = |tag: &str| "x".repeat(6_000) + tag;
        let mut messages = vec![
            serde_json::json!({"role": "user", "content": "original task"}),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "a1"}]}),
            serde_json::json!({"role": "tool", "tool_call_id": "a1", "content": big("A")}),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "a2"}]}),
            serde_json::json!({"role": "tool", "tool_call_id": "a2", "content": big("B")}),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "a3"}]}),
            serde_json::json!({"role": "tool", "tool_call_id": "a3", "content": big("C")}),
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "a4"}]}),
            serde_json::json!({"role": "tool", "tool_call_id": "a4", "content": big("D")}),
        ];
        // A comfortable budget changes nothing.
        assert!(!compact_messages(&mut messages, f64::MAX));
        assert!(messages.iter().any(|m| m["content"].as_str() == Some(big("A").as_str())));

        // A tiny budget: the oldest tool body is blanked first, then whole
        // rounds drop from the front until the estimate fits.
        let changed = compact_messages(&mut messages, 600.0);
        assert!(changed);
        assert_eq!(messages[0]["content"], "original task", "the original request is kept");
        // Nothing may be over budget after compaction (the tail is small).
        assert!(estimate_tokens_messages(&messages) <= 600.0 * 4.0 + 500.0);
        // Structural consistency: every tool message directly follows an
        // assistant message carrying its tool_calls, so the API never sees
        // an orphaned tool result.
        for index in 1..messages.len() {
            if messages[index]["role"] == "tool" {
                let previous = &messages[index - 1];
                assert_eq!(previous["role"], "assistant");
                assert!(previous["tool_calls"].is_array(), "orphaned tool result at {index}");
            }
        }
        // The newest tool body must have survived verbatim.
        assert!(
            messages.iter().any(|m| m["content"].as_str() == Some(big("D").as_str())),
            "the most recent tool result is kept"
        );
    }

    /// The cross-turn transcript cap follows the detected window. Runs under
    /// the mode lock so parallel integration tests cannot recalibrate the
    /// byte/token ratio (or rewrite the window) mid-assertion.
    #[test]
    fn transcript_cap_follows_the_detected_window() {
        let _guard = mode_lock();
        remember_window_limit(8192);
        assert_eq!(window_limit(), Some(8192));
        let expected = (8192.0 * COMPACTION_THRESHOLD * calibrated_chars_per_token()) as usize;
        assert_eq!(transcript_char_cap(), expected);
        // Sanity bounds regardless of calibration state.
        assert!((10_000..=84_000).contains(&transcript_char_cap()));
    }

    /// End-to-end self-heal: the mock rejects the first request with a
    /// context-length error, so the client must learn the 8192-token window
    /// from the error text, compact, retry, and still finish the turn.
    #[test]
    fn context_overflow_error_is_learned_and_retried() {
        let _guard = mode_lock();
        if !python_available() {
            eprintln!("skipping: python3 not available");
            return;
        }
        use std::io::BufRead as _;
        let mut server = std::process::Command::new("python3")
            .args(["tests/fixtures/openai_mock.py"])
            .env("MOCK_MODE", "limit")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start mock server");
        let address = {
            let stdout = server.stdout.take().expect("mock server stdout");
            let mut lines = BufReader::new(stdout).lines();
            lines.next().expect("mock server prints its address").unwrap()
        };

        // No explicit window: the /models probe on the mock must find it.
        let model = CustomModel::new(CustomConfig {
            name: "mock".into(),
            base_url: format!("http://{address}/v1"),
            api_key: "test-key".into(),
            model: "mock-model".into(),
            context_length: None,
        });
        let (sink, stream) = eb_api::AgentSink::channel();
        let handle = model.start("fill the window", sink);
        let mut text = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let finished = loop {
            if std::time::Instant::now() > deadline {
                break false;
            }
            match stream.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(eb_api::AgentEvent::Delta { text: value }) => text.push_str(&value),
                Ok(eb_api::AgentEvent::Failed { message }) => {
                    panic!("the overflow should self-heal, got: {message}");
                }
                Ok(eb_api::AgentEvent::Finished) => break true,
                Ok(_) => {}
                Err(_) => break false,
            }
        };
        handle.cancel();
        let _ = server.kill();
        assert!(finished, "the turn never finished after the overflow retry");
        assert_eq!(text, "recovered after compaction");
    }

    /// The composer permission mode and the EB_PERMISSION_AUTO env are
    /// process-global; the tests that touch them serialize on this lock so
    /// parallel tests cannot answer each other's prompts.
    fn mode_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap()
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
