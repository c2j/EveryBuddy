//! Custom OpenAI-compatible model answer source.
//!
//! A deliberately small adapter: one streamed `/chat/completions` request per
//! turn through `curl` (SSE in, `AgentEvent::Delta` out), so custom endpoints
//! work without adding a TLS/HTTP stack to the GTK host. Non-SSE answers
//! (servers that ignore `stream`, plain JSON errors) still parse.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eb_api::{AgentEvent, AgentSink, AnswerHandle, AnswerSource};
use eb_api::model::Block;

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
        let base = self.config.base_url.trim().trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        }
    }
}

impl AnswerSource for CustomModel {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        if let Err(error) = validate(&self.config) {
            let _ = sink.send(AgentEvent::Failed { message: error });
            return Box::new(CustomHandle { cancelled: Arc::new(AtomicBool::new(false)), child: Arc::new(Mutex::new(None)) });
        }
        let _ = sink.send(AgentEvent::Started { name: self.config.name.clone() });
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": true
        });
        let endpoint = self.endpoint();
        let key = self.config.api_key.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let child_for_handle = child.clone();
        let cancel = child.clone();
        let cancelled_for_worker = cancelled.clone();
        std::thread::Builder::new()
            .name("evobuddy-custom-model".into())
            .spawn(move || {
                let mut command = Command::new("curl");
                command
                    .args([
                        "-sS",
                        "-N",
                        "--max-time",
                        "300",
                        "-X",
                        "POST",
                        // The JSON body arrives on stdin; without `-d @-` the
                        // request goes out with an empty body and every
                        // endpoint answers with a request error.
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
                    let _ = sink.send(AgentEvent::Failed { message: "cannot start curl".into() });
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

                // Stream the SSE lines as they arrive: every
                // `choices[0].delta.content` becomes a Delta the transcript
                // appends live. A server that ignores `stream` (or fails) is
                // answered with one plain JSON body, which the fallback below
                // still parses.
                let stdout = child
                    .lock()
                    .ok()
                    .and_then(|mut slot| slot.as_mut().and_then(|p| p.stdout.take()));
                let mut streamed = String::new();
                let mut raw = String::new();
                let mut stream_error = String::new();
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
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                            continue;
                        };
                        if let Some(message) = value["error"]["message"].as_str() {
                            stream_error = message.to_string();
                            break;
                        }
                        if let Some(delta) =
                            value["choices"][0]["delta"]["content"].as_str()
                        {
                            if !delta.is_empty() {
                                streamed.push_str(delta);
                                let _ = sink.send(AgentEvent::Delta { text: delta.to_string() });
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

                // Streaming worked: the deltas already went out.
                if !streamed.is_empty() && stream_error.is_empty() {
                    let _ = sink.send(AgentEvent::Finished);
                    return;
                }

                // Fallbacks: an in-band stream error, or a server that
                // answered with one plain JSON body (stream ignored).
                let error = if !stream_error.is_empty() {
                    stream_error
                } else {
                    parse_response(&raw).err().unwrap_or_else(|| {
                        "empty response".into()
                    })
                };
                let detail = errors.lines().last().unwrap_or("").trim().to_string();
                let message =
                    if detail.is_empty() { error } else { format!("{error}: {detail}") };
                let _ = sink.send(AgentEvent::Failed { message });
            })
            .ok();
        let _ = cancel;
        Box::new(CustomHandle { cancelled, child: child_for_handle })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_accepts_base_v1_and_full_paths() {
        let mut config = CustomConfig { base_url: "https://example.test".into(), model: "m".into(), ..Default::default() };
        let model = CustomModel::new(config.clone());
        assert_eq!(model.endpoint(), "https://example.test/v1/chat/completions");
        config.base_url = "https://example.test/v1".into();
        assert_eq!(CustomModel::new(config.clone()).endpoint(), "https://example.test/v1/chat/completions");
        config.base_url = "https://example.test/v1/chat/completions".into();
        assert_eq!(CustomModel::new(config).endpoint(), "https://example.test/v1/chat/completions");
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
        use std::io::{BufRead, BufReader};
        let mut server = std::process::Command::new("python3")
            .args(["tests/fixtures/openai_mock.py"])
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
