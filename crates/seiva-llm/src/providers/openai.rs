//! The OpenAI Chat Completions API and every server that speaks it: OpenAI,
//! Azure OpenAI, vLLM, Ollama, Google Gemini's compatible endpoint, or an
//! OpenAI-compatible gateway the organisation already runs (a LiteLLM proxy, for
//! one — reached as an upstream on the network, never linked).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::sse::{event_stream, parse_arguments, Piece, SseEvent};
use super::{json, send, u64_at};
use crate::stream::EventStream;
use crate::transport::{Endpoint, HeaderValue, HttpRequest, HttpTransport, MAX_RESPONSE_BYTES};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, Content, CredentialSource, Embedder, Embeddings,
    LlmError, Role, StopReason, ToolChoice, Usage,
};

/// The public OpenAI API.
pub const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1";
/// The credential the OpenAI preset reads.
pub const OPENAI_CREDENTIAL: &str = "OPENAI_API_KEY";
/// Google Gemini's OpenAI-compatible endpoint.
#[cfg(feature = "gemini")]
pub const GEMINI_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
/// The credential the Gemini preset reads.
#[cfg(feature = "gemini")]
pub const GEMINI_CREDENTIAL: &str = "GEMINI_API_KEY";

/// Which body field carries the token cap. OpenAI's own API wants
/// `max_completion_tokens`; most compatible servers still read `max_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenLimitField {
    /// `max_tokens`.
    MaxTokens,
    /// `max_completion_tokens`.
    MaxCompletionTokens,
}

/// A provider for an OpenAI-compatible server.
#[derive(Clone)]
pub struct OpenAiCompatible {
    name: String,
    endpoint: Endpoint,
    credential: Option<String>,
    limit_field: TokenLimitField,
    transport: Arc<dyn HttpTransport>,
    credentials: Arc<dyn CredentialSource>,
}

impl OpenAiCompatible {
    /// A server at `endpoint` (the base that holds `/chat/completions`), sending no
    /// key — a local model. Add one with [`with_credential`](Self::with_credential).
    pub fn new(
        name: impl Into<String>,
        endpoint: Endpoint,
        transport: Arc<dyn HttpTransport>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Self {
        Self {
            name: name.into(),
            endpoint,
            credential: None,
            limit_field: TokenLimitField::MaxTokens,
            transport,
            credentials,
        }
    }

    /// The public OpenAI API, with the key in `OPENAI_API_KEY`.
    pub fn openai(
        transport: Arc<dyn HttpTransport>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Self {
        Self::new(
            "openai",
            Endpoint::https(OPENAI_ENDPOINT).expect("the default endpoint is valid"),
            transport,
            credentials,
        )
        .with_credential(OPENAI_CREDENTIAL)
        .with_token_limit_field(TokenLimitField::MaxCompletionTokens)
    }

    /// Google Gemini through its OpenAI-compatible endpoint, with the key in
    /// `GEMINI_API_KEY`.
    #[cfg(feature = "gemini")]
    pub fn gemini(
        transport: Arc<dyn HttpTransport>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Self {
        Self::new(
            "gemini",
            Endpoint::https(GEMINI_ENDPOINT).expect("the default endpoint is valid"),
            transport,
            credentials,
        )
        .with_credential(GEMINI_CREDENTIAL)
    }

    /// Send `Authorization: Bearer <credential>`.
    #[must_use]
    pub fn with_credential(mut self, name: impl Into<String>) -> Self {
        self.credential = Some(name.into());
        self
    }

    /// Choose the token-cap field.
    #[must_use]
    pub fn with_token_limit_field(mut self, field: TokenLimitField) -> Self {
        self.limit_field = field;
        self
    }

    async fn post(&self, path: &str, body: &Value) -> Result<HttpRequest, LlmError> {
        let body = serde_json::to_vec(body).map_err(|e| LlmError::Decode(e.to_string()))?;
        let mut http = HttpRequest::post_json(self.endpoint.join(path), body);
        if let Some(name) = &self.credential {
            let key = self.credentials.get(name).await?;
            http = http.header(
                "authorization",
                HeaderValue::sensitive(format!("Bearer {}", key.expose())),
            );
        }
        Ok(http)
    }
}

#[async_trait]
impl ChatProvider for OpenAiCompatible {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, model: &str, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let http = self
            .post(
                "chat/completions",
                &request_body(model, request, false, self.limit_field),
            )
            .await?;
        let body = send(&*self.transport, http)
            .await?
            .collect(MAX_RESPONSE_BYTES)
            .await?;
        parse_response(&json(&body)?)
    }

    async fn stream(&self, model: &str, request: &ChatRequest) -> Result<EventStream, LlmError> {
        let http = self
            .post(
                "chat/completions",
                &request_body(model, request, true, self.limit_field),
            )
            .await?;
        let response = send(&*self.transport, http).await?;
        Ok(event_stream(response.body, map_event))
    }
}

#[async_trait]
impl Embedder for OpenAiCompatible {
    async fn embed(&self, model: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        let http = self
            .post("embeddings", &json!({"model": model, "input": inputs}))
            .await?;
        let body = send(&*self.transport, http)
            .await?
            .collect(MAX_RESPONSE_BYTES)
            .await?;
        parse_embeddings(&json(&body)?, inputs.len())
    }
}

/// The JSON body of a Chat Completions request.
pub(crate) fn request_body(
    model: &str,
    request: &ChatRequest,
    stream: bool,
    limit: TokenLimitField,
) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    let limit_key = match limit {
        TokenLimitField::MaxTokens => "max_tokens",
        TokenLimitField::MaxCompletionTokens => "max_completion_tokens",
    };
    body.insert(limit_key.into(), json!(request.max_tokens));
    body.insert("messages".into(), Value::Array(messages(request)));
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = &request.tool_choice {
        let choice = match choice {
            ToolChoice::Auto => json!("auto"),
            ToolChoice::Required => json!("required"),
            ToolChoice::None => json!("none"),
            ToolChoice::Tool(name) => json!({"type": "function", "function": {"name": name}}),
        };
        body.insert("tool_choice".into(), choice);
    }
    if let Some(t) = request.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !request.stop.is_empty() {
        body.insert("stop".into(), json!(request.stop));
    }
    if stream {
        body.insert("stream".into(), json!(true));
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Value::Object(body)
}

/// The neutral conversation as Chat Completions messages: the system prompt
/// first, tool results as `tool` messages, tool calls on the assistant message.
fn messages(request: &ChatRequest) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = &request.system {
        out.push(json!({"role": "system", "content": system}));
    }
    for m in &request.messages {
        match m.role {
            Role::User => {
                let mut parts = Vec::new();
                for part in &m.content {
                    match part {
                        Content::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            let content = if *is_error {
                                format!("[error] {content}")
                            } else {
                                content.clone()
                            };
                            out.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}));
                        }
                        Content::Text { text } => parts.push(json!({"type": "text", "text": text})),
                        Content::Image { media_type, data } => {
                            parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{media_type};base64,{data}")}}));
                        }
                        Content::ToolUse { .. } => {}
                    }
                }
                if parts.is_empty() {
                    continue;
                }
                let all_text = parts.iter().all(|p| p["type"] == "text");
                let content = if all_text {
                    json!(parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<String>())
                } else {
                    Value::Array(parts)
                };
                out.push(json!({"role": "user", "content": content}));
            }
            Role::Assistant => {
                let text: String = m
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                let calls: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::ToolUse { id, name, input } => {
                            Some(json!({"id": id, "type": "function", "function": {"name": name, "arguments": input.to_string()}}))
                        }
                        _ => None,
                    })
                    .collect();
                let mut msg = Map::new();
                msg.insert("role".into(), json!("assistant"));
                msg.insert(
                    "content".into(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        json!(text)
                    },
                );
                if !calls.is_empty() {
                    msg.insert("tool_calls".into(), Value::Array(calls));
                }
                out.push(Value::Object(msg));
            }
        }
    }
    out
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "content_filter" => StopReason::Refusal,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Chat Completions usage, normalised: cached prompt tokens move out of
/// `input_tokens` into `cache_read_tokens`.
fn usage(u: &Value) -> Usage {
    let prompt = u64_at(u, "/prompt_tokens");
    let cached = u64_at(u, "/prompt_tokens_details/cached_tokens").min(prompt);
    Usage {
        input_tokens: prompt - cached,
        output_tokens: u64_at(u, "/completion_tokens"),
        cache_read_tokens: cached,
        cache_write_tokens: 0,
    }
}

/// A whole Chat Completions response.
pub(crate) fn parse_response(v: &Value) -> Result<ChatResponse, LlmError> {
    let choice = v
        .pointer("/choices/0")
        .ok_or_else(|| LlmError::Decode("the response has no `choices`".into()))?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    let mut content = Vec::new();
    let refusal = message
        .get("refusal")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    if let Some(text) = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        content.push(Content::text(text));
    } else if let Some(r) = refusal {
        content.push(Content::text(r));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            content.push(Content::ToolUse {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input: parse_arguments(
                    call.pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
            });
        }
    }
    let stop = if refusal.is_some() {
        StopReason::Refusal
    } else {
        stop_reason(
            choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .unwrap_or("stop"),
        )
    };
    let model = v.get("model").and_then(Value::as_str).unwrap_or_default();
    Ok(ChatResponse::new(
        model,
        content,
        stop,
        usage(v.get("usage").unwrap_or(&Value::Null)),
    ))
}

/// One chunk of a Chat Completions stream. Text is block 0; tool call `i` is
/// block `i + 1`.
pub(crate) fn map_event(ev: SseEvent) -> Result<Vec<Piece>, LlmError> {
    if ev.data.trim() == "[DONE]" {
        return Ok(vec![Piece::Done]);
    }
    let v: Value = serde_json::from_str(&ev.data).map_err(|e| LlmError::Decode(e.to_string()))?;
    if let Some(err) = v.get("error") {
        return Err(LlmError::status(
            500,
            err.get("message")
                .and_then(Value::as_str)
                .unwrap_or("stream error"),
        ));
    }
    let mut out = Vec::new();
    if let Some(model) = v.get("model").and_then(Value::as_str) {
        out.push(Piece::Model(model.to_owned()));
    }
    if let Some(choice) = v.pointer("/choices/0") {
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            out.push(Piece::Text {
                block: 0,
                text: text.to_owned(),
            });
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let block = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize + 1;
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    out.push(Piece::ToolStart {
                        block,
                        id: id.to_owned(),
                        name: call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    });
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    out.push(Piece::ToolArgs {
                        block,
                        json: args.to_owned(),
                    });
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            out.push(Piece::Stop(stop_reason(reason)));
        }
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        out.push(Piece::Usage(usage(u)));
    }
    Ok(out)
}

fn parse_embeddings(v: &Value, expected: usize) -> Result<Embeddings, LlmError> {
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Decode("the response has no `data`".into()))?;
    let mut indexed: Vec<(usize, Vec<f32>)> = data
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let index = d
                .get("index")
                .and_then(Value::as_u64)
                .map_or(i, |x| x as usize);
            let vector = d
                .get("embedding")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_f64)
                        .map(|x| x as f32)
                        .collect()
                })
                .unwrap_or_default();
            (index, vector)
        })
        .collect();
    indexed.sort_by_key(|(i, _)| *i);
    if indexed.len() != expected {
        return Err(LlmError::Decode(format!(
            "expected {expected} embeddings, got {}",
            indexed.len()
        )));
    }
    let usage = Usage {
        input_tokens: u64_at(v, "/usage/prompt_tokens"),
        ..Usage::default()
    };
    Ok(Embeddings::new(
        indexed.into_iter().map(|(_, v)| v).collect(),
        usage,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ToolSpec};

    #[test]
    fn the_request_body_maps_system_tools_and_tool_results() {
        let req = ChatRequest::new(100)
            .system("sys")
            .user("hi")
            .message(Message::new(
                Role::Assistant,
                vec![Content::ToolUse {
                    id: "c1".into(),
                    name: "lookup".into(),
                    input: json!({"id": 7}),
                }],
            ))
            .message(Message::new(
                Role::User,
                vec![Content::tool_result("c1", "boom", true)],
            ))
            .tool(ToolSpec::new("lookup", "find", json!({"type": "object"})))
            .tool_choice(ToolChoice::Tool("lookup".into()));
        let body = request_body("gpt-x", &req, true, TokenLimitField::MaxCompletionTokens);
        assert_eq!(body["max_completion_tokens"], 100);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0], json!({"role": "system", "content": "sys"}));
        assert_eq!(msgs[1], json!({"role": "user", "content": "hi"}));
        assert_eq!(msgs[2]["content"], Value::Null);
        assert_eq!(
            msgs[2]["tool_calls"][0]["function"]["arguments"],
            "{\"id\":7}"
        );
        assert_eq!(
            msgs[3],
            json!({"role": "tool", "tool_call_id": "c1", "content": "[error] boom"})
        );
        assert_eq!(
            body["tools"][0]["function"]["parameters"],
            json!({"type": "object"})
        );
        assert_eq!(
            body["tool_choice"],
            json!({"type": "function", "function": {"name": "lookup"}})
        );
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
    }

    #[test]
    fn a_whole_response_maps_tool_calls_and_cached_tokens() {
        let v = json!({
            "model": "gpt-x",
            "choices": [{"message": {"content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "lookup", "arguments": "{\"id\":7}"}},
                {"id": "c2", "type": "function", "function": {"name": "lookup", "arguments": "{broken"}}
            ]}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 20, "prompt_tokens_details": {"cached_tokens": 900}}
        });
        let r = parse_response(&v).unwrap();
        assert_eq!(r.stop_reason, StopReason::ToolUse);
        let calls = r.tool_uses();
        assert_eq!(calls[0].2, &json!({"id": 7}));
        assert_eq!(calls[1].2, &Value::String("{broken".into()));
        assert_eq!(
            r.usage,
            Usage {
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 900,
                cache_write_tokens: 0
            }
        );
    }

    #[test]
    fn a_refusal_is_reported_as_one() {
        let v = json!({"choices": [{"message": {"content": null, "refusal": "I can't help with that."}, "finish_reason": "stop"}]});
        let r = parse_response(&v).unwrap();
        assert_eq!(r.stop_reason, StopReason::Refusal);
        assert_eq!(r.text(), "I can't help with that.");
    }

    #[test]
    fn stream_chunks_map_to_pieces() {
        let ev = |data: &str| SseEvent {
            event: None,
            data: data.into(),
        };
        assert_eq!(
            map_event(ev(r#"{"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"lookup","arguments":""}}]}}]}"#)).unwrap(),
            vec![
                Piece::Model("m".into()),
                Piece::ToolStart { block: 1, id: "c1".into(), name: "lookup".into() },
                Piece::ToolArgs { block: 1, json: String::new() },
            ]
        );
        assert_eq!(map_event(ev("[DONE]")).unwrap(), vec![Piece::Done]);
        assert!(map_event(ev(
            r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#
        ))
        .unwrap()
        .contains(&Piece::Usage(Usage {
            input_tokens: 3,
            output_tokens: 2,
            ..Usage::default()
        })));
    }

    #[test]
    fn embeddings_come_back_in_input_order() {
        let v = json!({"data": [{"index": 1, "embedding": [0.5]}, {"index": 0, "embedding": [0.25]}], "usage": {"prompt_tokens": 4}});
        let e = parse_embeddings(&v, 2).unwrap();
        assert_eq!(e.vectors, vec![vec![0.25], vec![0.5]]);
        assert!(parse_embeddings(&v, 3).is_err());
    }
}
