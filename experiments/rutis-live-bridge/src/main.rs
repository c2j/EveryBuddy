//! EvoBuddy-specific A2 bridge. It mounts rutis 0.12's generic dsh agent
//! composition, keeps one live Session, and exposes the real messageFeedback
//! service to the WebKit component host over local HTTP.

use std::sync::Arc;
use std::time::Duration;

use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use aimux_llm::service::PartStream;
use aimux_llm::{LlmService, LlmServiceError, ModelBrief, StreamRequest};
use rutis::Ctx;
use rutis_dsh::agent;
use serde_json::{json, Value};
use tiny_http::{Header, Response, Server};

const PORT: u16 = 39472;

struct Scripted;

#[async_trait::async_trait]
impl LlmService for Scripted {
    async fn stream(&self, req: StreamRequest) -> Result<PartStream, LlmServiceError> {
        let last = req.options.messages.last();
        let parts = if last.and_then(|message| message.role.as_deref()) == Some("tool") {
            let answer = format!("The code is {}.", last.unwrap().text);
            vec![
                StreamPart::TextDelta { id: "t".into(), delta: answer, provider_metadata: None },
                finish(FinishReasonUnified::Stop),
            ]
        } else {
            vec![
                StreamPart::ToolCall {
                    tool_call_id: "call-1".into(),
                    tool_name: "lookup".into(),
                    input: json!({ "q": "m2" }),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                },
                finish(FinishReasonUnified::ToolCalls),
            ]
        };
        Ok(Box::pin(futures::stream::iter(parts.into_iter().map(Ok))))
    }

    async fn list_models(
        &self,
        provider: &str,
        _key: Option<&str>,
    ) -> Result<Vec<ModelBrief>, LlmServiceError> {
        Ok(vec![ModelBrief { id: format!("{provider}-chat"), ..Default::default() }])
    }
}

fn finish(reason: FinishReasonUnified) -> StreamPart {
    StreamPart::Finish {
        finish_reason: FinishReason { unified: reason, raw: None },
        usage: Usage::default(),
        provider_metadata: None,
    }
}

async fn mount() -> Ctx {
    let ctx = Ctx::root().unwrap();
    rutis_dsh::provide_agent_aimux(&ctx, Arc::new(Scripted)).unwrap();
    let view = ctx.plugin(agent::Plugin::new(agent::Config {
        invariants: Default::default(),
        typert: Default::default(),
        llm: Default::default(),
        session: Default::default(),
        projection: Default::default(),
        system_prompt: Default::default(),
        tools: Default::default(),
        agents: Default::default(),
        agent_loop: Default::default(),
        adapter: agent::AdapterConfig {
            providers: [(
                "evobuddy".to_string(),
                agent::Route { provider: None, api_key_env: None, display_name: None },
            )]
            .into_iter()
            .collect(),
        },
        llm_probe: Default::default(),
        agent_probe: Default::default(),
        session_persistence: agent::SessionPersistenceConfig {
            root: "/tmp/evobuddy-rutis-sessions".into(),
            compression: None,
        },
        message_feedback: agent::MessageFeedbackConfig { max_note_bytes: 8192.0 },
    }));
    (&view).await.unwrap();
    ctx
}

fn json_response(value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = value.to_string();
    Response::from_string(body)
        .with_header(Header::from_bytes(&b"content-type"[..], &b"application/json"[..]).unwrap())
        .with_header(Header::from_bytes(&b"access-control-allow-origin"[..], &b"*"[..]).unwrap())
        .with_header(Header::from_bytes(&b"access-control-allow-headers"[..], &b"content-type"[..]).unwrap())
        .with_header(Header::from_bytes(&b"access-control-allow-methods"[..], &b"GET, POST, OPTIONS"[..]).unwrap())
}

fn serve(
    server: Server,
    handle: tokio::runtime::Handle,
    feedback: Arc<agent::MessageFeedbackService>,
    session_json: String,
) {
    for mut request in server.incoming_requests() {
        if request.method().as_str() == "OPTIONS" {
            let _ = request.respond(json_response(json!({})));
            continue;
        }
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("/");
        let session_id = url
            .split("sessionId=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or("")
            .to_string();
        let session_id = if session_id.is_empty() {
            let parsed: Value = serde_json::from_str(&session_json).unwrap_or_default();
            parsed["sessionId"].as_str().unwrap_or_default().to_string()
        } else {
            session_id
        };
        let response = match (request.method().as_str(), path) {
            ("GET", "/session") => json_response(
                serde_json::from_str(&session_json).unwrap_or_else(|_| json!({"sessionId": null, "messages": []})),
            ),
            ("GET", "/feedback") => {
                let result = handle.block_on(feedback.list(&agent::MessageFeedbackListRequest {
                    session_id: agent::SessionId(session_id.clone()),
                }));
                json_response(result.unwrap_or_else(|error| json!({"ok": false, "error": error.to_string()})))
            }
            ("POST", "/feedback/put") => {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let input: Value = serde_json::from_str(&body).unwrap_or_default();
                let rating = match input["rating"].as_str() {
                    Some("negative") => agent::MessageFeedbackRating::Negative,
                    _ => agent::MessageFeedbackRating::Positive,
                };
                let category = input["category"].as_str().and_then(|value| match value {
                    "task-result" => Some(agent::MessageFeedbackPutRequestCategory::TaskResult),
                    "instruction-following" => Some(agent::MessageFeedbackPutRequestCategory::InstructionFollowing),
                    "product-interaction" => Some(agent::MessageFeedbackPutRequestCategory::ProductInteraction),
                    "service-stability" => Some(agent::MessageFeedbackPutRequestCategory::ServiceStability),
                    "resource-cost" => Some(agent::MessageFeedbackPutRequestCategory::ResourceCost),
                    "security-privacy-permission" => Some(agent::MessageFeedbackPutRequestCategory::SecurityPrivacyPermission),
                    "other" => Some(agent::MessageFeedbackPutRequestCategory::Other),
                    _ => None,
                });
                let result = handle.block_on(feedback.put(&agent::MessageFeedbackPutRequest {
                    session_id: agent::SessionId(session_id.clone()),
                    message_id: agent::MessageId(input["messageId"].as_str().unwrap_or_default().to_string()),
                    rating,
                    note: input["note"].as_str().map(str::to_string),
                    category,
                    if_version: input["ifVersion"].as_str().map(agent::MessageFeedbackVersion::from),
                }));
                json_response(result.unwrap_or_else(|error| json!({"ok": false, "error": error.to_string()})))
            }
            ("POST", "/feedback/delete") => {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let input: Value = serde_json::from_str(&body).unwrap_or_default();
                let result = handle.block_on(feedback.delete(&agent::MessageFeedbackDeleteRequest {
                    session_id: agent::SessionId(session_id.clone()),
                    message_id: agent::MessageId(input["messageId"].as_str().unwrap_or_default().to_string()),
                    if_version: agent::MessageFeedbackVersion::from(input["ifVersion"].as_str().unwrap_or_default()),
                }));
                json_response(result.unwrap_or_else(|error| json!({"ok": false, "error": error.to_string()})))
            }
            _ => Response::from_string("not found").with_status_code(404),
        };
        let _ = request.respond(response);
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let ctx = mount().await;
    let probe = ctx.get::<agent::AgentProbe>().unwrap();
    let feedback = ctx.get::<agent::MessageFeedbackService>().unwrap();
    let session = probe
        .run_persistent("evobuddy", "fake", "What is the code name?")
        .await
        .unwrap();
    eprintln!("evobuddy-bridge: {session}");
    eprintln!("evobuddy-bridge: listening on http://127.0.0.1:{PORT}");
    let server = Server::http(("127.0.0.1", PORT)).expect("bind feedback bridge");
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || serve(server, handle, feedback, session));
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
