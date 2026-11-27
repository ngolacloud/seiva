//! The Anthropic Messages shape: `POST /v1/messages` (and `count_tokens`).
//!
//! The `model` field names a gateway **route**, never a provider model.

use std::collections::{BTreeMap, VecDeque};

use seiva_llm::{
    ChatRequest, ChatResponse, Content, Message, Role, StopReason, StreamEvent, ToolChoice,
    ToolSpec,
};
use serde_json::{json, Value};

use super::{Limits, Parsed};

/// Read a Messages request.
pub(crate) fn parse(body: &Value, limits: &Limits) -> Result<Parsed, String> {
    let route = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or("`model` (the gateway route) is required")?
        .to_owned();
    let max_tokens = limits.max_tokens(body.get("max_tokens").and_then(Value::as_u64))?;
    let mut request = ChatRequest::new(max_tokens);
    match body.get("system") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => request = request.system(s.clone()),
        Some(Value::Array(blocks)) => {
            let text: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            request = request.system(text.join("\n\n"));
        }
        Some(_) => return Err("`system` must be a string or text blocks".into()),
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("`messages` is required")?;
    for m in messages {
        let role = match m.get("role").and_then(Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            other => return Err(format!("unknown message role `{}`", other.unwrap_or("?"))),
        };
        let parts = match m.get("content") {
            Some(Value::String(s)) => vec![Content::text(s.clone())],
            Some(Value::Array(blocks)) => blocks
                .iter()
                .map(block)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect(),
            _ => return Err("`content` must be a string or blocks".into()),
        };
        request = request.message(Message::new(role, parts));
    }
    for t in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or("a tool needs a name")?;
        let schema = t.get("input_schema").cloned().ok_or_else(|| {
            format!("tool `{name}` has no `input_schema` (server tools are not supported)")
        })?;
        request = request.tool(ToolSpec::new(
            name,
            t.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            schema,
        ));
    }
    if let Some(choice) = body.get("tool_choice") {
        let choice = match choice.get("type").and_then(Value::as_str) {
            Some("any") => ToolChoice::Required,
            Some("none") => ToolChoice::None,
            Some("tool") => ToolChoice::Tool(
                choice
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            _ => ToolChoice::Auto,
        };
        request = request.tool_choice(choice);
    }
    if let Some(t) = body.get("temperature").and_then(Value::as_f64) {
        request = request.temperature(t as f32);
    }
    for s in body
        .get("stop_sequences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        request = request.stop(s);
    }
    Ok(Parsed {
        route,
        request,
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        include_usage: true,
    })
}

/// One content block; thinking blocks from an earlier turn are dropped.
fn block(b: &Value) -> Result<Option<Content>, String> {
    let s = |p: &str| {
        b.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(match b.get("type").and_then(Value::as_str) {
        Some("text") => Some(Content::text(s("/text"))),
        Some("image") => {
            if b.pointer("/source/type").and_then(Value::as_str) != Some("base64") {
                return Err(
                    "only base64 images are accepted (the gateway does not fetch URLs)".into(),
                );
            }
            Some(Content::Image {
                media_type: s("/source/media_type"),
                data: s("/source/data"),
            })
        }
        Some("tool_use") => Some(Content::ToolUse {
            id: s("/id"),
            name: s("/name"),
            input: b.get("input").cloned().unwrap_or_else(|| json!({})),
        }),
        Some("tool_result") => {
            let content = match b.get("content") {
                Some(Value::String(t)) => t.clone(),
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            Some(Content::tool_result(
                s("/tool_use_id"),
                content,
                b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            ))
        }
        Some("thinking") | Some("redacted_thinking") => None,
        other => {
            return Err(format!(
                "unsupported content block `{}`",
                other.unwrap_or("?")
            ))
        }
    })
}

pub(crate) fn stop_reason(stop: &StopReason) -> &'static str {
    match stop {
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
        StopReason::StopSequence => "stop_sequence",
        StopReason::Refusal => "refusal",
        _ => "end_turn",
    }
}

fn usage_json(r: &ChatResponse) -> Value {
    let u = r.usage;
    json!({
        "input_tokens": u.input_tokens,
        "output_tokens": u.output_tokens,
        "cache_read_input_tokens": u.cache_read_tokens,
        "cache_creation_input_tokens": u.cache_write_tokens,
    })
}

fn content_json(c: &Content) -> Option<Value> {
    match c {
        Content::Text { text } => Some(json!({"type": "text", "text": text})),
        Content::ToolUse { id, name, input } => {
            Some(json!({"type": "tool_use", "id": id, "name": name, "input": input}))
        }
        _ => None,
    }
}

/// A whole Messages response.
pub(crate) fn render(route: &str, id: &str, r: &ChatResponse) -> Value {
    json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": route,
        "content": r.content.iter().filter_map(content_json).collect::<Vec<_>>(),
        "stop_reason": stop_reason(&r.stop_reason),
        "stop_sequence": Value::Null,
        "usage": usage_json(r),
    })
}

/// Turns gateway events into Messages stream events `(event, data)`. Text blocks
/// stream as released; tool calls are sent whole at the end, unmasked.
pub(crate) struct Eventer {
    route: String,
    id: String,
    started: bool,
    /// Gateway content index → open block number.
    open: BTreeMap<usize, usize>,
    next_block: usize,
}

impl Eventer {
    pub fn new(route: &str, id: &str) -> Self {
        Self {
            route: route.to_owned(),
            id: id.to_owned(),
            started: false,
            open: BTreeMap::new(),
            next_block: 0,
        }
    }

    pub fn on_event(&mut self, event: &StreamEvent) -> VecDeque<(&'static str, String)> {
        let mut out = VecDeque::new();
        if !self.started {
            self.started = true;
            out.push_back((
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": self.id, "type": "message", "role": "assistant", "model": self.route,
                    "content": [], "stop_reason": Value::Null, "stop_sequence": Value::Null,
                    "usage": {"input_tokens": 0, "output_tokens": 0},
                }})
                .to_string(),
            ));
        }
        match event {
            StreamEvent::TextDelta { index, text } => {
                let block = match self.open.get(index) {
                    Some(b) => *b,
                    None => {
                        let b = self.next_block;
                        self.next_block += 1;
                        self.open.insert(*index, b);
                        out.push_back((
                            "content_block_start",
                            json!({"type": "content_block_start", "index": b, "content_block": {"type": "text", "text": ""}}).to_string(),
                        ));
                        b
                    }
                };
                out.push_back((
                    "content_block_delta",
                    json!({"type": "content_block_delta", "index": block, "delta": {"type": "text_delta", "text": text}}).to_string(),
                ));
            }
            StreamEvent::Completed(r) => {
                for (_, b) in std::mem::take(&mut self.open) {
                    out.push_back((
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": b}).to_string(),
                    ));
                }
                for (id, name, input) in r.tool_uses() {
                    let b = self.next_block;
                    self.next_block += 1;
                    out.push_back((
                        "content_block_start",
                        json!({"type": "content_block_start", "index": b, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}})
                            .to_string(),
                    ));
                    out.push_back((
                        "content_block_delta",
                        json!({"type": "content_block_delta", "index": b, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}})
                            .to_string(),
                    ));
                    out.push_back((
                        "content_block_stop",
                        json!({"type": "content_block_stop", "index": b}).to_string(),
                    ));
                }
                out.push_back((
                    "message_delta",
                    json!({"type": "message_delta", "delta": {"stop_reason": stop_reason(&r.stop_reason), "stop_sequence": Value::Null},
                           "usage": usage_json(r)})
                    .to_string(),
                ));
                out.push_back(("message_stop", json!({"type": "message_stop"}).to_string()));
            }
            _ => {}
        }
        out
    }

    pub fn on_error(&mut self, message: &str) -> VecDeque<(&'static str, String)> {
        VecDeque::from([(
            "error",
            json!({"type": "error", "error": {"type": "api_error", "message": message}})
                .to_string(),
        )])
    }
}
