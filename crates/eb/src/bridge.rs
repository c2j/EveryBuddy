//! Owns the EvoBuddy-specific rutis live bridge process.
//!
//! The binary lives in `experiments/rutis-live-bridge` and serves the real dsh
//! messageFeedback service on 127.0.0.1:39472. This module is the single owner:
//! Settings starts/stops it, previews ensure it is up, and app shutdown stops it.

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

const ADDR: &str = "127.0.0.1:39472";

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
/// The one session the bridge is currently serving. Projection is global, so
/// only this task may consume it; otherwise selecting an old task would
/// overwrite its transcript with the live session's messages.
static ACTIVE_SESSION: Mutex<Option<String>> = Mutex::new(None);

pub fn healthy() -> bool {
    TcpStream::connect_timeout(
        &ADDR.parse().expect("bridge address"),
        Duration::from_millis(150),
    )
    .is_ok()
}

fn binary() -> PathBuf {
    std::env::var("EVOBUDDY_BRIDGE_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("experiments/rutis-live-bridge/target/debug/evobuddy-rutis-bridge")
        })
}

fn dsh_credentials() -> Vec<(String, String)> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let path = std::path::PathBuf::from(home).join(".dsh/.credentials.yaml");
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut values = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        for (key, env) in [
            ("AIHUBMIX_API_KEY:", "AIHUBMIX_API_KEY"),
            ("ZAI_CODING_CN_API_KEY:", "ZAI_API_KEY"),
        ] {
            if let Some(value) = line.strip_prefix(key) {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    values.push((env.to_string(), value.to_string()));
                }
            }
        }
    }
    values
}

/// Start the bridge if it is not already healthy. Returns whether a process
/// was spawned by this call.
pub fn start() -> Result<bool, String> {
    if healthy() {
        return Ok(false);
    }
    let mut slot = CHILD.lock().unwrap();
    if let Some(child) = slot.as_mut() {
        if child.try_wait().ok().flatten().is_none() {
            return Ok(false);
        }
    }
    let mut command = Command::new(binary());
    command
        .env("EVOBUDDY_LIVE_BRIDGE", "1")
        .env("AIMUX_PROVIDER", std::env::var("AIMUX_PROVIDER").unwrap_or_else(|_| "aihubmix".into()))
        .env("AIMUX_MODEL", std::env::var("AIMUX_MODEL").unwrap_or_else(|_| "deepseek-v4.1-flash".into()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in dsh_credentials() {
        command.env(key, value);
    }
    let child = command.spawn().map_err(|error| {
        eprintln!("bridge: cannot start: {error}");
        format!("cannot start bridge: {error}")
    })?;
    *slot = Some(child);
    for _ in 0..40 {
        if healthy() {
            return Ok(true);
        }
        if let Some(child) = slot.as_mut() {
            if child.try_wait().ok().flatten().is_some() {
                return Err("bridge exited during startup".into());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("bridge: did not become healthy");
    Err("bridge did not become healthy".into())
}

/// Ensure the bridge is up; errors are reported by Settings/preview callers.
pub fn ensure() {
    let _ = start();
}

pub fn stop() {
    let mut slot = CHILD.lock().unwrap();
    if let Some(mut child) = slot.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn status() -> &'static str {
    if healthy() {
        "running · http://127.0.0.1:39472"
    } else {
        "stopped"
    }
}

/// Mirror the active EvoBuddy conversation into the live dsh session before
/// opening the feedback panel. The bridge turns the real user text and the real
/// assistant answer into one dsh session, then the feedback surface addresses
/// that session's assistant message.
pub fn bind_conversation(prompt: &str, answer: &str) -> Result<String, String> {
    let body = serde_json::json!({ "prompt": prompt, "answer": answer }).to_string();
    let mut stream = TcpStream::connect(ADDR).map_err(|error| error.to_string())?;
    let request = format!(
        "POST /session/append HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).map_err(|error| error.to_string())?;
    let mut response = String::new();
    use std::io::Read;
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    if response.contains("\"ok\":true") || response.contains("\"ok\": true") {
        let marker = "\"sessionId\":\"";
        if let Some(start) = response.find(marker) {
            let rest = &response[start + marker.len()..];
            let end = rest.find('"').unwrap_or(0);
            return Ok(rest[..end].to_string());
        }
        Ok(String::new())
    } else {
        Err(response)
    }
}

/// Create the empty dsh session that will back one EvoBuddy task.
pub fn create_session(session_id: &str) -> Result<(), String> {
    let body = serde_json::json!({ "sessionId": session_id }).to_string();
    let mut stream = TcpStream::connect(ADDR).map_err(|error| error.to_string())?;
    let request = format!(
        "POST /session/create HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).map_err(|error| error.to_string())?;
    let mut response = String::new();
    use std::io::Read;
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    if response.contains("\"ok\":true") || response.contains("\"ok\": true") {
        *ACTIVE_SESSION.lock().unwrap() = Some(session_id.to_string());
        Ok(())
    } else {
        Err(response)
    }
}

/// Append one completed turn to the task's retained dsh session.
pub fn append_turn(
    session_id: &str,
    prompt: &str,
    answer: &str,
    tools: &[(String, String, String)],
) -> Result<(), String> {
    let tools: Vec<_> = tools
        .iter()
        .map(|(name, arguments, result)| {
            serde_json::json!({ "name": name, "arguments": arguments, "result": result })
        })
        .collect();
    let body = serde_json::json!({
        "sessionId": session_id,
        "prompt": prompt,
        "answer": answer,
        "tools": tools,
    })
    .to_string();
    let mut stream = TcpStream::connect(ADDR).map_err(|error| error.to_string())?;
    let request = format!(
        "POST /session/append HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).map_err(|error| error.to_string())?;
    let mut response = String::new();
    use std::io::Read;
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    if response.contains("\"ok\":true") || response.contains("\"ok\": true") {
        Ok(())
    } else {
        Err(response)
    }
}

/// Read the retained session's user/assistant text in order.
pub fn session_turns() -> Vec<(String, String)> {
    let Ok(mut stream) = TcpStream::connect(ADDR) else { return Vec::new() };
    let request = "GET /session HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return Vec::new();
    }
    let mut response = String::new();
    use std::io::Read;
    if stream.read_to_string(&mut response).is_err() {
        return Vec::new();
    }
    let Some(body) = response.split("\r\n\r\n").nth(1) else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    value["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let role = message["role"].as_str()?;
                    if role != "user" && role != "assistant" {
                        return None;
                    }
                    let text = message["content"]
                        .as_array()?
                        .iter()
                        .filter_map(|block| block["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("");
                    Some((role.to_string(), text))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The session currently served by the bridge, if any.
pub fn active_session() -> Option<String> {
    ACTIVE_SESSION.lock().unwrap().clone()
}

/// Resume a persisted task session after an app restart.
pub fn resume_session(session_id: &str) -> Result<(), String> {
    let body = serde_json::json!({ "sessionId": session_id }).to_string();
    let mut stream = TcpStream::connect(ADDR).map_err(|error| error.to_string())?;
    let request = format!(
        "POST /session/resume HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).map_err(|error| error.to_string())?;
    let mut response = String::new();
    use std::io::Read;
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    if response.contains("\"ok\":true") || response.contains("\"ok\": true") {
        *ACTIVE_SESSION.lock().unwrap() = Some(session_id.to_string());
        Ok(())
    } else {
        Err(response)
    }
}

/// Ordered session projection including tool rows.
pub fn session_projection() -> Vec<serde_json::Value> {
    let Ok(mut stream) = TcpStream::connect(ADDR) else { return Vec::new() };
    let request = "GET /projection HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() { return Vec::new() }
    let mut response = String::new();
    use std::io::Read;
    if stream.read_to_string(&mut response).is_err() { return Vec::new() }
    let Some(body) = response.split("\r\n\r\n").nth(1) else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    value["items"].as_array().cloned().unwrap_or_default()
}

fn post_json(path: &str, body: serde_json::Value) -> Result<String, String> {
    let body = body.to_string();
    let mut stream = TcpStream::connect(ADDR).map_err(|error| error.to_string())?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).map_err(|error| error.to_string())?;
    let mut response = String::new();
    use std::io::Read;
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    Ok(response)
}

pub fn begin_turn(prompt: &str) -> Result<(), String> {
    let response = post_json("/session/begin", serde_json::json!({ "prompt": prompt }))?;
    if response.contains("\"ok\":true") { Ok(()) } else { Err(response) }
}

pub fn append_attempt(text: &str) -> Result<(), String> {
    let response = post_json("/session/attempt", serde_json::json!({ "text": text }))?;
    if response.contains("\"ok\":true") { Ok(()) } else { Err(response) }
}

pub fn finish_turn(answer: &str, tools: &[(String, String, String)]) -> Result<(), String> {
    let tools: Vec<_> = tools.iter().map(|(name, arguments, result)| serde_json::json!({
        "name": name, "arguments": arguments, "result": result
    })).collect();
    let response = post_json("/session/finish", serde_json::json!({ "answer": answer, "tools": tools }))?;
    if response.contains("\"ok\":true") { Ok(()) } else { Err(response) }
}

pub fn run_agent(prompt: &str) -> Result<Vec<(String, String)>, String> {
    let response = post_json("/agent/turn", serde_json::json!({ "prompt": prompt }))?;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or(&response);
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    if value["ok"] != true {
        return Err(value["error"].as_str().unwrap_or("dsh agent turn failed").to_string());
    }
    Ok(value["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let role = message["role"].as_str()?.to_string();
                    let text = message["content"].as_array()?.iter()
                        .filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("");
                    Some((role, text))
                })
                .collect()
        })
        .unwrap_or_default())
}
