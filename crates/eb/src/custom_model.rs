//! Custom OpenAI-compatible model answer source.
//!
//! This is a deliberately small adapter: one non-streaming
//! `/chat/completions` request per turn through `curl`, so custom endpoints can
//! be used without adding a TLS/HTTP stack to the GTK host.

use std::io::{Read, Write};
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
            "stream": false
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
                        "--max-time",
                        "120",
                        "-X",
                        "POST",
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
                if let Some(stdin) = process.stdin.as_mut() {
                    let _ = stdin.write_all(body.to_string().as_bytes());
                }
                if let Ok(mut slot) = child.lock() {
                    *slot = Some(process);
                }
                let mut output = String::new();
                let mut errors = String::new();
                if let Ok(mut slot) = child.lock() {
                    if let Some(process) = slot.as_mut() {
                        if let Some(mut stdout) = process.stdout.take() {
                            let _ = stdout.read_to_string(&mut output);
                        }
                        if let Some(mut stderr) = process.stderr.take() {
                            let _ = stderr.read_to_string(&mut errors);
                        }
                        let _ = process.wait();
                    }
                }
                if cancelled_for_worker.load(Ordering::SeqCst) {
                    let _ = sink.send(AgentEvent::Failed { message: "Cancelled".into() });
                    return;
                }
                match parse_response(&output) {
                    Ok(text) => {
                        let _ = sink.send(AgentEvent::Block { block: Block::Text(text) });
                        let _ = sink.send(AgentEvent::Finished);
                    }
                    Err(error) => {
                        let detail = errors.lines().last().unwrap_or("").trim();
                        let message = if detail.is_empty() { error } else { format!("{error}: {detail}") };
                        let _ = sink.send(AgentEvent::Failed { message });
                    }
                }
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
}
