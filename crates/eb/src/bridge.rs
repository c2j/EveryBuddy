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
        .map_err(|error| format!("cannot start bridge: {error}"))?;
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
pub fn bind_conversation(prompt: &str, answer: &str) -> Result<(), String> {
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
        Ok(())
    } else {
        Err(response)
    }
}
