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
    let child = Command::new(binary())
        .env("EVOBUDDY_LIVE_BRIDGE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
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
        Ok(())
    } else {
        Err(response)
    }
}

/// Append one completed turn to the task's retained dsh session.
pub fn append_turn(session_id: &str, prompt: &str, answer: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "sessionId": session_id,
        "prompt": prompt,
        "answer": answer,
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
