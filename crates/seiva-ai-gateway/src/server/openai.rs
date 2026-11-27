//! The OpenAI Chat Completions shape: `POST /v1/chat/completions`.
//!
//! The `model` field names a gateway **route**, never a provider model.

use std::collections::VecDeque;

use seiva_llm::{
    ChatRequest, ChatResponse, Content, Message, Role, StopReason, StreamEvent, ToolChoice,
    ToolSpec,
};
use serde_json::{json, Map, Value};

use super::{arguments, data_url, Limits, Parsed};

/// Read a Chat Completions request.
pub(crate) fn parse(body: &Value, limits: &Limits) -> Result<Parsed, String> {
    let route = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or("`model` (the gateway route) is required")?
        .to_owned();
    let requested = body
        .get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64);
    let max_tokens = limits.max_tokens(requested)?;
    let mut request = ChatRequest::new(max_tokens);

    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("`messages` is required")?;
    let mut system: Vec<String> = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "system" | "developer" => system.push(text_of(m.get("content"))?),
            "user" => {
                let parts = user_parts(m.get("content"))?;
                request = request.message(Message::new(Role::User, parts));
            }
            "assistant" => {
                let mut parts = Vec::new();
                let text = match m.get("content") {
                    None | Some(Value::Null) => String::new(),
                    other => text_of(other)?,
                };
                if !text.is_empty() {
                    parts.push(Content::text(text));
                }
                for call in m
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    parts.push(Content::ToolUse {
                        id: str_at(call, "/id").to_owned(),
                        name: str_at(call, "/function/name").to_owned(),
                        input: arguments(str_at(call, "/function/arguments")),
                    });
                }
                request = request.message(Message::new(Role::Assistant, parts));
            }
            "tool" => {
                let result = Content::tool_result(
                    str_at(m, "/tool_call_id"),
                    text_of(m.get("content"))?,
                    false,
                );
                // Consecutive tool results travel in one user message.
                match request.messages.last_mut() {
                    Some(last)
                        if last.role == Role::User
                            && last
                                .content
                                .iter()
                                .all(|c| matches!(c, Content::ToolResult { .. })) =>
                    {
                        last.content.push(result)
                    }
                    _ => request = request.message(Message::new(Role::User, vec![result])),
                }
            }
            other => return Err(format!("unknown message role `{other}`")),
        }
    }
    if !system.is_empty() {
        request = request.system(system.join("\n\n"));
    }

    for t in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if t.get("type").and_then(Value::as_str).unwrap_or("function") != "function" {
            return Err("only `function` tools are supported".into());
        }
        let f = t.get("function").ok_or("a tool needs `function`")?;
        let name = f
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or("a tool needs a name")?;
        request = request.tool(ToolSpec::new(
            name,
            f.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            f.get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
        ));
    }
    if let Some(choice) = body.get("tool_choice") {
        let choice = match choice {
            Value::String(s) if s == "auto" => ToolChoice::Auto,
            Value::String(s) if s == "none" => ToolChoice::None,
            Value::String(s) if s == "required" => ToolChoice::Required,
            v => ToolChoice::Tool(str_at(v, "/function/name").to_owned()),
        };
        request = request.tool_choice(choice);
    }
    if let Some(t) = body.get("temperature").and_then(Value::as_f64) {
        request = request.temperature(t as f32);
    }
    match body.get("stop") {
        Some(Value::String(s)) => request = request.stop(s.clone()),
        Some(Value::Array(items)) => {
            for s in items.iter().filter_map(Value::as_str) {
                request = request.stop(s);
            }
        }
        _ => {}
    }
    Ok(Parsed {
        route,
        request,
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        include_usage: body
            .pointer("/stream_options/include_usage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> &'a str {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The text of a `content` that is a string or an array of text parts.
fn text_of(content: Option<&Value>) -> Result<String, String> {
    match content {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(parts)) => Ok(parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("")),
        Some(_) => Err("`content` must be a string or an array of parts".into()),
    }
}

fn user_parts(content: Option<&Value>) -> Result<Vec<Content>, String> {
    match content {
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => Ok(Content::text(str_at(p, "/text"))),
                Some("image_url") => {
                    let (media_type, data) = data_url(str_at(p, "/image_url/url"))?;
                    Ok(Content::Image { media_type, data })
                }
                other => Err(format!(
                    "unsupported content part `{}`",
                    other.unwrap_or("?")
                )),
            })
            .collect(),
        other => Ok(vec![Content::text(text_of(other)?)]),
    }
}

pub(crate) fn finish_reason(stop: &StopReason) -> &'static str {
    match stop {
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
        StopReason::Refusal => "content_filter",
        _ => "stop",
    }
}

fn usage_json(r: &ChatResponse) -> Value {
    let u = r.usage;
    let prompt = u.total_input();
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": u.output_tokens,
        "total_tokens": prompt + u.output_tokens,
        "prompt_tokens_details": {"cached_tokens": u.cache_read_tokens},
    })
}

fn tool_calls(r: &ChatResponse) -> Vec<Value> {
    r.tool_uses()
        .into_iter()
        .map(|(id, name, input)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": input.to_string()}}))
        .collect()
}

/// A whole Chat Completions response.
pub(crate) fn render(route: &str, id: &str, created: u64, r: &ChatResponse) -> Value {
    let text = r.text();
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    let calls = tool_calls(r);
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": route,
        "choices": [{"index": 0, "message": Value::Object(message), "finish_reason": finish_reason(&r.stop_reason)}],
        "usage": usage_json(r),
    })
}

/// Turns gateway events into Chat Completions chunks. Text streams as it is
/// released; tool calls are sent whole at the end, unmasked (their argument
/// fragments stream masked from the gateway and are not forwarded).
pub(crate) struct Chunker {
    route: String,
    id: String,
    created: u64,
    include_usage: bool,
    started: bool,
}

impl Chunker {
    pub fn new(route: &str, id: &str, created: u64, include_usage: bool) -> Self {
        Self {
            route: route.to_owned(),
            id: id.to_owned(),
            created,
            include_usage,
            started: false,
        }
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> String {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.route,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
        .to_string()
    }

    /// The `data:` payloads for one gateway event, in order. `[DONE]` ends.
    pub fn on_event(&mut self, event: &StreamEvent) -> VecDeque<String> {
        let mut out = VecDeque::new();
        if !self.started {
            self.started = true;
            out.push_back(self.chunk(json!({"role": "assistant", "content": ""}), None));
        }
        match event {
            StreamEvent::TextDelta { text, .. } => {
                out.push_back(self.chunk(json!({"content": text}), None))
            }
            StreamEvent::Completed(r) => {
                let calls: Vec<Value> = tool_calls(r)
                    .into_iter()
                    .enumerate()
                    .map(|(i, mut c)| {
                        c["index"] = json!(i);
                        c
                    })
                    .collect();
                if !calls.is_empty() {
                    out.push_back(self.chunk(json!({"tool_calls": calls}), None));
                }
                out.push_back(self.chunk(json!({}), Some(finish_reason(&r.stop_reason))));
                if self.include_usage {
                    out.push_back(
                        json!({
                            "id": self.id, "object": "chat.completion.chunk", "created": self.created,
                            "model": self.route, "choices": [], "usage": usage_json(r),
                        })
                        .to_string(),
                    );
                }
                out.push_back("[DONE]".into());
            }
            _ => {}
        }
        out
    }

    /// The payloads for an error that ends the stream.
    pub fn on_error(&mut self, message: &str) -> VecDeque<String> {
        VecDeque::from([
            json!({"error": {"message": message, "type": "api_error"}}).to_string(),
            "[DONE]".into(),
        ])
    }
}
