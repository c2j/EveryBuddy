//! The core's outbound channel.
//!
//! The core cannot spawn processes, cannot reach the network, and should not
//! grow a second way to touch the host. One import covers everything WASI does
//! not: logging for now, and later the callbacks that let a Rust plugin consult
//! a Cordis service owned by the host.
//!
//! Reply buffers are allocated **in the core's linear memory** by the host,
//! using the core's own `kernel_alloc`. That keeps ownership in one place: the
//! core always frees what it reads.
use crate::protocol::{HostCall, HostReply};

#[cfg(target_arch = "wasm32")]
mod transport {
    use crate::mem;

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_call(ptr: *const u8, len: usize) -> *mut u8;
    }

    /// Send `payload` to the host and read its framed reply.
    ///
    /// The host allocates the reply in our memory through `kernel_alloc`, so
    /// release is ours too. `None` means the host declined to answer.
    pub fn exchange(payload: &[u8]) -> Option<Vec<u8>> {
        let ptr = unsafe { host_call(payload.as_ptr(), payload.len()) };
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { mem::take_frame(ptr) })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    /// Native builds have no host channel. Tests exercise the core's logic
    /// directly, so a call here is recorded on stderr in the same shape the
    /// Node host prints, and answered `ok`.
    pub fn exchange(payload: &[u8]) -> Option<Vec<u8>> {
        match serde_json::from_slice::<serde_json::Value>(payload) {
            Ok(call) if call["method"] == "log" => {
                eprintln!(
                    "  [core:{}] {}",
                    call["params"]["level"].as_str().unwrap_or("info"),
                    call["params"]["message"].as_str().unwrap_or(""),
                );
            }
            Ok(call) => eprintln!("  [core] call: {call}"),
            Err(_) => eprintln!("  [core] {}", String::from_utf8_lossy(payload)),
        }
        Some(br#"{"ok":true,"result":null}"#.to_vec())
    }
}

/// Call one host method. Returns the host's result payload, or `None` when the
/// host rejected the call or the reply did not decode.
pub fn call(method: &str, params: serde_json::Value) -> Option<serde_json::Value> {
    let payload = serde_json::to_vec(&HostCall { method, params }).ok()?;
    let bytes = transport::exchange(&payload)?;
    let reply: HostReply = serde_json::from_slice(&bytes).ok()?;
    if reply.ok {
        Some(reply.result)
    } else {
        None
    }
}

/// Write one line to the host's log. Failures are dropped: a log call that
/// cannot be delivered must not become the reason a request fails.
pub fn log(level: &str, message: &str) {
    let _ = call(
        "log",
        serde_json::json!({ "level": level, "message": message }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_transport_answers_ok() {
        assert!(call("log", serde_json::json!({ "message": "x" })).is_some());
    }

    #[test]
    fn log_does_not_panic_without_a_host() {
        log("info", "hello");
    }
}
