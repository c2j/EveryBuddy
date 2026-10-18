//! Anthropic Messages 流式客户端(与 dsh llm-deepseek 适配器同一线协议)。
//!
//! 只覆盖本演示用到的面:一条 system、一组 tools、消息历史、SSE 增量。
//! 线格式逐字段对齐 `packages/llm/llm-deepseek/src/serialize.ts` 与
//! `translate.ts`(已验证的实现),而不是猜测协议:
//!
//! - POST `{base}/v1/messages`,头 `x-api-key` + `anthropic-version: 2023-06-01`
//! - body 必带 `max_tokens` 与 `thinking`;assistant 历史里的 thinking 块带回
//!   signature(input_json_delta 的 tool 实参按累计 JSON 还原)
//! - 工具结果以 **user 角色**的 `tool_result` 块回传
//!
//! 阻塞式 I/O 是刻意选择:内核本来就是同步驱动的,一条请求一个线程最直白。
use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{Value, json};

/// 一次模型回复:线格式块(可直接进历史)、工具调用、停止原因与用量。
pub struct Turn {
    /// assistant 消息的 wire content 数组(text/thinking/tool_use)。
    pub content: Value,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 模型请求的一次工具调用。
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 原始 JSON 字符串(与模型产出一致)。
    pub arguments: String,
}

/// 流式过程中的可见增量。
pub enum Delta {
    Text(String),
    Thinking(String),
}

/// 一个 DeepSeek(Anthropic Messages)路由。
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    api_key: String,
    model: String,
    /// Anthropic 必填的输出上限;dsh 目录里 deepseek-chat 默认 8192。
    max_tokens: u64,
}

impl Client {
    /// 一条路由:端点、凭据、模型与输出上限。
    pub fn new(base_url: String, api_key: String, model: String, max_tokens: u64) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            agent,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            max_tokens,
        }
    }

    /// 发起一次流式请求;增量经 `on_delta` 即时回调,终态汇总为 [`Turn`]。
    ///
    /// `messages` 是 wire 数组(user/assistant/user-tool_result),由调用方
    /// 用 [`assistant_content`]/[`tool_result_message`]/[`user_message`] 组装。
    pub fn stream(
        &self,
        system: &str,
        messages: &Value,
        tools: &Value,
        mut on_delta: impl FnMut(&Delta),
    ) -> Result<Turn, String> {
        let mut body = json!({
            "model": self.model,
            "stream": true,
            "messages": messages,
            "max_tokens": self.max_tokens,
            // 与 dsh 适配器的默认裁决一致:thinking 开、effort high。
            "thinking": { "type": "enabled" },
            "output_config": { "effort": "high" },
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if tools.as_array().is_some_and(|tools| !tools.is_empty()) {
            body["tools"] = tools.clone();
        }

        let response = self
            .agent
            .post(format!("{}/v1/messages", self.base_url))
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send_json(&body)
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.into_body().read_to_string().unwrap_or_default();
            let head: String = text.chars().take(400).collect();
            return Err(format!("HTTP {status}: {head}"));
        }
        let reader = response.into_body().into_reader();
        self.consume(reader, &mut on_delta)
    }

    /// 解析 SSE 帧,翻译成块与终态(逐事件对齐 dsh 的 translate.ts)。
    fn consume(
        &self,
        reader: impl std::io::Read,
        on_delta: &mut impl FnMut(&Delta),
    ) -> Result<Turn, String> {
        // 线格式块(text/thinking/tool_use)与其余产物。
        let mut content: Vec<Value> = Vec::new();
        struct Open {
            wire_index: usize,
            kind: String,
            text: String,
            signature: String,
            call: Option<ToolCall>,
        }
        let mut open: Vec<Open> = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut saw_stop = false;

        let mut data_line = String::new();
        let mut line = String::new();
        let mut reader = BufReader::new(reader);
        loop {
            line.clear();
            let read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if trimmed.is_empty() {
                if data_line.is_empty() {
                    continue;
                }
                let event: Value = serde_json::from_str(data_line.as_str())
                    .map_err(|e| format!("SSE 数据不是 JSON: {e}: {}", &data_line[..data_line.len().min(120)]))?;
                data_line.clear();
                let kind = event["type"].as_str().unwrap_or_default().to_string();
                match kind.as_str() {
                    "message_start" => {
                        input_tokens = event["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
                    }
                    "content_block_start" => {
                        let block = &event["content_block"];
                        let wire_index = event["index"].as_u64().unwrap_or(0) as usize;
                        match block["type"].as_str().unwrap_or_default() {
                            "text" => open.push(Open {
                                wire_index,
                                kind: "text".into(),
                                text: block["text"].as_str().unwrap_or_default().to_string(),
                                signature: String::new(),
                                call: None,
                            }),
                            "thinking" => open.push(Open {
                                wire_index,
                                kind: "thinking".into(),
                                text: block["thinking"].as_str().unwrap_or_default().to_string(),
                                signature: String::new(),
                                call: None,
                            }),
                            "tool_use" => open.push(Open {
                                wire_index,
                                kind: "tool-call".into(),
                                text: String::new(),
                                signature: String::new(),
                                call: Some(ToolCall {
                                    id: block["id"].as_str().unwrap_or_default().to_string(),
                                    name: block["name"].as_str().unwrap_or_default().to_string(),
                                    arguments: String::new(),
                                }),
                            }),
                            other => return Err(format!("不支持的响应块类型: {other}")),
                        }
                    }
                    "content_block_delta" => {
                        let wire_index = event["index"].as_u64().unwrap_or(0) as usize;
                        let slot = open
                            .iter_mut()
                            .find(|b| b.wire_index == wire_index)
                            .ok_or_else(|| "delta 落在未开启的块上".to_string())?;
                        let delta = &event["delta"];
                        match delta["type"].as_str().unwrap_or_default() {
                            "text_delta" => {
                                let text = delta["text"].as_str().unwrap_or_default();
                                slot.text.push_str(text);
                                if slot.kind == "text" {
                                    on_delta(&Delta::Text(text.to_string()));
                                }
                            }
                            "thinking_delta" => {
                                let text = delta["thinking"].as_str().unwrap_or_default();
                                slot.text.push_str(text);
                                on_delta(&Delta::Thinking(text.to_string()));
                            }
                            "signature_delta" => {
                                slot.signature.push_str(delta["signature"].as_str().unwrap_or_default());
                            }
                            "input_json_delta" => {
                                if let Some(call) = slot.call.as_mut() {
                                    call.arguments.push_str(delta["partial_json"].as_str().unwrap_or_default());
                                }
                            }
                            _ => {} // 未知增量按协议容忍
                        }
                    }
                    "content_block_stop" => {
                        let wire_index = event["index"].as_u64().unwrap_or(0) as usize;
                        let slot = open
                            .iter()
                            .find(|b| b.wire_index == wire_index)
                            .ok_or_else(|| "stop 落在未开启的块上".to_string())?;
                        match slot.kind.as_str() {
                            "text" if !slot.text.is_empty() => {
                                content.push(json!({ "type": "text", "text": slot.text }));
                            }
                            "thinking" => {
                                let mut block = json!({ "type": "thinking", "thinking": slot.text });
                                if !slot.signature.is_empty() {
                                    block["signature"] = json!(slot.signature);
                                }
                                content.push(block);
                            }
                            "tool-call" => {
                                if let Some(call) = &slot.call {
                                    content.push(json!({
                                        "type": "tool_use",
                                        "id": call.id,
                                        "name": call.name,
                                        "input": serde_json::from_str::<Value>(&call.arguments)
                                            .unwrap_or(Value::Object(Default::default())),
                                    }));
                                }
                            }
                            _ => {} // 空文本块不入历史
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                            stop_reason = Some(reason.to_string());
                        }
                        if let Some(tokens) = event["usage"]["output_tokens"].as_u64() {
                            output_tokens = tokens;
                        }
                    }
                    "message_stop" => {
                        saw_stop = true;
                        break;
                    }
                    "ping" | "error" => {}
                    _ => {} // Anthropic 允许新增事件类型
                }
                continue;
            }
            if let Some(payload) = trimmed.strip_prefix("data:") {
                data_line.push_str(payload.trim_start());
            }
            // `event:` 行忽略:协议语义都在 data 的 type 字段里。
        }

        if !saw_stop {
            return Err("流在 message_stop 之前结束".to_string());
        }
        let stop_reason = stop_reason.unwrap_or_else(|| "end_turn".to_string());
        let tool_calls = open
            .into_iter()
            .filter_map(|slot| slot.call)
            .collect();
        Ok(Turn {
            content: Value::Array(content),
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
        })
    }
}

/// user 一轮文本输入。
pub fn user_message(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "text", "text": text }] })
}

/// assistant 历史:线格式块数组原样回填(含 thinking 签名与 tool_use)。
pub fn assistant_message(content: Value) -> Value {
    json!({ "role": "assistant", "content": content })
}

/// 工具结果块:Anthropic 要求同帧 user 消息里带齐上一帧的**全部** tool_result。
pub fn tool_result_block(call_id: &str, text: &str, is_error: bool) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": call_id,
        "content": [{ "type": "text", "text": text }],
        "is_error": is_error,
    })
}
