//! The Anthropic Messages API (`POST /v1/messages`), whole and streamed.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::sse::{event_stream, Piece, SseEvent};
use super::{json, send, u64_at};
use crate::stream::EventStream;
use crate::transport::{Endpoint, HeaderValue, HttpRequest, HttpTransport, MAX_RESPONSE_BYTES};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, Content, CredentialSource, LlmError, Role, StopReason,
    ToolChoice, Usage,
};

/// The public Anthropic API.
pub const DEFAULT_ENDPOINT: &str = "https://api.anthropic.com";
/// The credential read by default.
pub const DEFAULT_CREDENTIAL: &str = "ANTHROPIC_API_KEY";
/// The API version header sent.
pub const API_VERSION: &str = "2023-06-01";

/// A provider for the Anthropic Messages API.
#[derive(Clone)]
pub struct Anthropic {
    name: String,
    endpoint: Endpoint,
    credential: String,
    transport: Arc<dyn HttpTransport>,
    credentials: Arc<dyn CredentialSource>,
}

impl Anthropic {
    /// The public API, with the key in `ANTHROPIC_API_KEY`.
    pub fn new(transport: Arc<dyn HttpTransport>, credentials: Arc<dyn CredentialSource>) -> Self {
        Self {
            name: "anthropic".into(),
            endpoint: Endpoint::https(DEFAULT_ENDPOINT).expect("the default endpoint is valid"),
            credential: DEFAULT_CREDENTIAL.into(),
            transport,
            credentials,
        }
    }

    /// Another endpoint with the same API (a regional or private deployment).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Read the key from another credential name.
    #[must_use]
    pub fn with_credential(mut self, name: impl Into<String>) -> Self {
        self.credential = name.into();
        self
    }

    /// Name it differently (two Anthropic accounts in one router).
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    async fn request(
        &self,
        model: &str,
        request: &ChatRequest,
        stream: bool,
    ) -> Result<HttpRequest, LlmError> {
        let key = self.credentials.get(&self.credential).await?;
        let body = serde_json::to_vec(&request_body(model, request, stream))
            .map_err(|e| LlmError::Decode(e.to_string()))?;
        Ok(
            HttpRequest::post_json(self.endpoint.join("v1/messages"), body)
                .header("x-api-key", HeaderValue::sensitive(key.expose()))
                .header("anthropic-version", HeaderValue::plain(API_VERSION)),
        )
    }
}

#[async_trait]
impl ChatProvider for Anthropic {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, model: &str, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let http = self.request(model, request, false).await?;
        let body = send(&*self.transport, http)
            .await?
            .collect(MAX_RESPONSE_BYTES)
            .await?;
        parse_response(&json(&body)?)
    }

    async fn stream(&self, model: &str, request: &ChatRequest) -> Result<EventStream, LlmError> {
        let http = self.request(model, request, true).await?;
        let response = send(&*self.transport, http).await?;
        Ok(event_stream(response.body, map_event))
    }
}

/// The JSON body of a Messages request.
pub(crate) fn request_body(model: &str, request: &ChatRequest, stream: bool) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(request.max_tokens));
    if let Some(system) = &request.system {
        body.insert("system".into(), json!(system));
    }
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": match m.role { Role::User => "user", Role::Assistant => "assistant" },
                "content": m.content.iter().map(content_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    body.insert("messages".into(), Value::Array(messages));
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = &request.tool_choice {
        let choice = match choice {
            ToolChoice::Auto => json!({"type": "auto"}),
            ToolChoice::Required => json!({"type": "any"}),
            ToolChoice::None => json!({"type": "none"}),
            ToolChoice::Tool(name) => json!({"type": "tool", "name": name}),
        };
        body.insert("tool_choice".into(), choice);
    }
    if let Some(t) = request.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !request.stop.is_empty() {
        body.insert("stop_sequences".into(), json!(request.stop));
    }
    if stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

fn content_json(part: &Content) -> Value {
    match part {
        Content::Text { text } => json!({"type": "text", "text": text}),
        Content::Image { media_type, data } => {
            json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})
        }
        Content::ToolUse { id, name, input } => {
            json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        Content::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error})
        }
    }
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "tool_use" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        "refusal" => StopReason::Refusal,
        other => StopReason::Other(other.to_owned()),
    }
}

fn usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u64_at(u, "/input_tokens"),
        output_tokens: u64_at(u, "/output_tokens"),
        cache_read_tokens: u64_at(u, "/cache_read_input_tokens"),
        cache_write_tokens: u64_at(u, "/cache_creation_input_tokens"),
    }
}

/// A whole Messages response.
pub(crate) fn parse_response(v: &Value) -> Result<ChatResponse, LlmError> {
    let parts = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Decode("the response has no `content`".into()))?;
    let content = parts
        .iter()
        .filter_map(|p| match p.get("type").and_then(Value::as_str) {
            Some("text") => Some(Content::text(
                p.get("text").and_then(Value::as_str).unwrap_or_default(),
            )),
            Some("tool_use") => Some(Content::ToolUse {
                id: p
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input: p.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            // Thinking blocks and server tools are not part of the neutral model.
            _ => None,
        })
        .collect();
    let stop = stop_reason(
        v.get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn"),
    );
    let model = v.get("model").and_then(Value::as_str).unwrap_or_default();
    Ok(ChatResponse::new(
        model,
        content,
        stop,
        usage(v.get("usage").unwrap_or(&Value::Null)),
    ))
}

/// One event of a Messages stream.
pub(crate) fn map_event(ev: SseEvent) -> Result<Vec<Piece>, LlmError> {
    let v: Value = serde_json::from_str(&ev.data).map_err(|e| LlmError::Decode(e.to_string()))?;
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .or(ev.event.as_deref())
        .unwrap_or_default();
    let index = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(match kind {
        "message_start" => {
            let m = v.get("message").unwrap_or(&Value::Null);
            vec![
                Piece::Model(
                    m.get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ),
                Piece::Usage(usage(m.get("usage").unwrap_or(&Value::Null))),
            ]
        }
        "content_block_start" => {
            let b = v.get("content_block").unwrap_or(&Value::Null);
            match b.get("type").and_then(Value::as_str) {
                Some("text") => vec![Piece::Text {
                    block: index,
                    text: b
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }],
                Some("tool_use") => vec![Piece::ToolStart {
                    block: index,
                    id: b
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    name: b
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }],
                _ => vec![],
            }
        }
        "content_block_delta" => {
            let d = v.get("delta").unwrap_or(&Value::Null);
            match d.get("type").and_then(Value::as_str) {
                Some("text_delta") => vec![Piece::Text {
                    block: index,
                    text: d
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }],
                Some("input_json_delta") => vec![Piece::ToolArgs {
                    block: index,
                    json: d
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }],
                _ => vec![],
            }
        }
        "message_delta" => {
            let mut out = vec![Piece::Usage(usage(v.get("usage").unwrap_or(&Value::Null)))];
            if let Some(s) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                out.push(Piece::Stop(stop_reason(s)));
            }
            out
        }
        "message_stop" => vec![Piece::Done],
        "error" => {
            let kind = v
                .pointer("/error/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = match kind {
                "overloaded_error" => 529,
                "rate_limit_error" => 429,
                _ => 500,
            };
            return Err(LlmError::status(
                status,
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or(kind),
            ));
        }
        // ping, content_block_stop and anything new.
        _ => vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ToolSpec};

    #[test]
    fn the_request_body_follows_the_messages_api() {
        let req = ChatRequest::new(256)
            .system("be brief")
            .user("hi")
            .message(Message::new(
                Role::User,
                vec![Content::tool_result("t1", "42", false)],
            ))
            .tool(ToolSpec::new("lookup", "find", json!({"type": "object"})))
            .tool_choice(ToolChoice::Required)
            .stop("END");
        let body = request_body("claude-x", &req, true);
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["system"], "be brief");
        assert_eq!(
            body["messages"][0],
            json!({"role": "user", "content": [{"type": "text", "text": "hi"}]})
        );
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["input_schema"], json!({"type": "object"}));
        assert_eq!(body["tool_choice"], json!({"type": "any"}));
        assert_eq!(body["stop_sequences"], json!(["END"]));
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn a_whole_response_maps_text_tools_stop_and_cached_usage() {
        let v = json!({
            "model": "claude-x",
            "content": [
                {"type": "thinking", "thinking": "…"},
                {"type": "text", "text": "Let me look."},
                {"type": "tool_use", "id": "toolu_1", "name": "lookup", "input": {"id": 7}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 12, "output_tokens": 30, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 5}
        });
        let r = parse_response(&v).unwrap();
        assert_eq!(r.text(), "Let me look.");
        assert_eq!(
            r.tool_uses(),
            vec![("toolu_1", "lookup", &json!({"id": 7}))]
        );
        assert_eq!(r.stop_reason, StopReason::ToolUse);
        assert_eq!(
            r.usage,
            Usage {
                input_tokens: 12,
                output_tokens: 30,
                cache_read_tokens: 1000,
                cache_write_tokens: 5
            }
        );
    }

    #[test]
    fn stream_events_map_to_pieces_and_errors() {
        let ev = |data: &str| SseEvent {
            event: None,
            data: data.into(),
        };
        assert_eq!(
            map_event(ev(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\""}}"#)).unwrap(),
            vec![Piece::ToolArgs { block: 1, json: "{\"a\"".into() }]
        );
        assert_eq!(
            map_event(ev(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":9}}"#)).unwrap()[1],
            Piece::Stop(StopReason::MaxTokens)
        );
        assert_eq!(map_event(ev(r#"{"type":"ping"}"#)).unwrap(), vec![]);
        assert_eq!(
            map_event(ev(
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ))
            .unwrap_err(),
            LlmError::status(529, "Overloaded")
        );
    }
}
