//! The wire contract between the host and the core.
//!
//! Two directions, one encoding: every message is a JSON object, and the only
//! shape difference is which side owns the reply. Keeping one encoding means a
//! new language only has to implement JSON and the two buffer functions in
//! [`crate::abi`].
use serde::{Deserialize, Serialize};

/// A call from the host into the core.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// Echoed back so the host can match a reply to its call.
    pub id: u64,
    /// Dotted method name, for example `search.find`.
    pub method: String,
    /// Method-specific arguments. Absent arguments decode to `null`.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// The core's answer to one [`Request`].
#[derive(Debug, Serialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

impl Response {
    /// A successful reply carrying a serializable payload.
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        Self {
            id,
            ok: true,
            result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
            error: None,
        }
    }

    /// A rejected reply. `code` is stable for callers to branch on; `message`
    /// is for a human reading a log.
    pub fn fail(id: u64, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(Failure {
                code: code.to_string(),
                message: message.into(),
            }),
        }
    }
}

/// A structured failure. The host never has to parse `message` to decide what
/// to do; `code` carries the decision.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

/// A call from the core out to the host.
#[derive(Debug, Serialize)]
pub struct HostCall<'a> {
    pub method: &'a str,
    pub params: serde_json::Value,
}

/// The host's answer to one [`HostCall`].
#[derive(Debug, Deserialize)]
pub struct HostReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
}
