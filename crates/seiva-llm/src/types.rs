//! The provider-neutral conversation types: what a product sends to a model and
//! what comes back, whichever provider serves it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who wrote a [`Message`]. The system prompt is not a role here: it travels on
/// [`ChatRequest::system`], once per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person (or the product on their behalf). Tool results travel in a
    /// user message, as [`Content::ToolResult`] parts.
    User,
    /// The model.
    Assistant,
}

/// One part of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Content {
    /// Plain text.
    Text {
        /// The text.
        text: String,
    },
    /// An image, base64-encoded.
    Image {
        /// Its media type (`image/png`, `image/jpeg`…).
        media_type: String,
        /// The image bytes, base64.
        data: String,
    },
    /// The model asks to call a tool.
    ToolUse {
        /// The call id, echoed back by the matching [`Content::ToolResult`].
        id: String,
        /// The tool name.
        name: String,
        /// The arguments. A provider that returned arguments that are not valid
        /// JSON yields them as a JSON string, for the caller's validation to
        /// reject (never silently repaired).
        input: Value,
    },
    /// The result of a tool call, sent back to the model.
    ToolResult {
        /// The id of the [`Content::ToolUse`] this answers.
        tool_use_id: String,
        /// What the tool returned, as text.
        content: String,
        /// Whether the call failed.
        #[serde(default)]
        is_error: bool,
    },
}

impl Content {
    /// A text part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// A tool-result part.
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error,
        }
    }
}

/// One turn of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// Its parts, in order.
    pub content: Vec<Content>,
}

impl Message {
    /// A user message with one text part.
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![Content::text(text)],
        }
    }

    /// An assistant message with one text part.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![Content::text(text)],
        }
    }

    /// A message with the given role and parts.
    pub fn new(role: Role, content: Vec<Content>) -> Self {
        Self { role, content }
    }
}

/// A tool the model may call. The input schema is a JSON Schema object — for a
/// facade method, the `WireSchema` of its command (ADR 0007/0025).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// The tool name the model calls.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input_schema: Value,
}

impl ToolSpec {
    /// A tool spec.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// Whether, and which, tool the model must call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolChoice {
    /// The model decides (the default when tools are offered).
    Auto,
    /// The model must call some tool.
    Required,
    /// The model must not call a tool.
    None,
    /// The model must call this tool.
    Tool(String),
}

/// One request to a model. `max_tokens` is mandatory: an unbounded generation is
/// unbounded spend (OWASP LLM10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChatRequest {
    /// The system prompt. Never a place for secrets or credentials (OWASP LLM07).
    pub system: Option<String>,
    /// The conversation so far.
    pub messages: Vec<Message>,
    /// The tools the model may call.
    pub tools: Vec<ToolSpec>,
    /// Whether, and which, tool the model must call.
    pub tool_choice: Option<ToolChoice>,
    /// The cap on generated tokens.
    pub max_tokens: u32,
    /// Sampling temperature, if the product sets one.
    pub temperature: Option<f32>,
    /// Sequences that stop the generation.
    pub stop: Vec<String>,
}

impl ChatRequest {
    /// A request with no messages yet, capped at `max_tokens`.
    pub fn new(max_tokens: u32) -> Self {
        Self {
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            max_tokens,
            temperature: None,
            stop: Vec::new(),
        }
    }

    /// Set the system prompt.
    #[must_use]
    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Append a message.
    #[must_use]
    pub fn message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    /// Append a user text message.
    #[must_use]
    pub fn user(self, text: impl Into<String>) -> Self {
        self.message(Message::user(text))
    }

    /// Offer a tool.
    #[must_use]
    pub fn tool(mut self, tool: ToolSpec) -> Self {
        self.tools.push(tool);
        self
    }

    /// Set the tool choice.
    #[must_use]
    pub fn tool_choice(mut self, choice: ToolChoice) -> Self {
        self.tool_choice = Some(choice);
        self
    }

    /// Set the temperature.
    #[must_use]
    pub fn temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Add a stop sequence.
    #[must_use]
    pub fn stop(mut self, sequence: impl Into<String>) -> Self {
        self.stop.push(sequence.into());
        self
    }
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    /// It finished its answer.
    EndTurn,
    /// It hit [`ChatRequest::max_tokens`].
    MaxTokens,
    /// It wants to call a tool.
    ToolUse,
    /// It produced one of [`ChatRequest::stop`].
    StopSequence,
    /// The provider refused or filtered the answer.
    Refusal,
    /// Something the codec does not map, as the provider named it.
    Other(String),
}

/// Token usage of one call, normalised across providers.
///
/// `input_tokens` counts the input billed at the normal rate — **excluding** the
/// tokens read from or written to a prompt cache, which are counted apart (they
/// are priced differently). OpenAI reports cached tokens inside its prompt count;
/// the codec subtracts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens at the normal rate.
    pub input_tokens: u64,
    /// Generated tokens.
    pub output_tokens: u64,
    /// Input tokens read from a prompt cache.
    pub cache_read_tokens: u64,
    /// Input tokens written to a prompt cache.
    pub cache_write_tokens: u64,
}

impl Usage {
    /// Every input token, whatever its rate.
    pub fn total_input(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, rhs: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + rhs.input_tokens,
            output_tokens: self.output_tokens + rhs.output_tokens,
            cache_read_tokens: self.cache_read_tokens + rhs.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens + rhs.cache_write_tokens,
        }
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Usage) {
        *self = *self + rhs;
    }
}

/// The model's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChatResponse {
    /// The model that answered, as the provider named it.
    pub model: String,
    /// The answer's parts: text and tool calls, in order.
    pub content: Vec<Content>,
    /// Why it stopped.
    pub stop_reason: StopReason,
    /// What it cost, in tokens.
    pub usage: Usage,
    /// `provider/model` of the upstream that served it, set by the [`Router`](crate::Router)
    /// (`None` when a provider is called directly).
    pub upstream: Option<String>,
}

impl ChatResponse {
    /// A response. Providers and test doubles build one this way.
    pub fn new(
        model: impl Into<String>,
        content: Vec<Content>,
        stop_reason: StopReason,
        usage: Usage,
    ) -> Self {
        Self {
            model: model.into(),
            content,
            stop_reason,
            usage,
            upstream: None,
        }
    }

    /// All the text parts, joined.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls, in order: `(id, name, input)`.
    pub fn tool_uses(&self) -> Vec<(&str, &str, &Value)> {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
                _ => None,
            })
            .collect()
    }

    /// The answer as an assistant message, to append to the conversation.
    pub fn to_message(&self) -> Message {
        Message::new(Role::Assistant, self.content.clone())
    }
}

/// One event of a streamed answer. The stream always ends with
/// [`StreamEvent::Completed`], carrying the whole answer assembled from the
/// deltas — so a caller that only wants the result can skip the rest.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StreamEvent {
    /// More text for the content part at `index`.
    TextDelta {
        /// The content part this text belongs to.
        index: usize,
        /// The new text.
        text: String,
    },
    /// A tool call starts at `index`.
    ToolUseStart {
        /// The content part of the call.
        index: usize,
        /// The call id.
        id: String,
        /// The tool name.
        name: String,
    },
    /// More of the JSON arguments of the tool call at `index`.
    ToolInputDelta {
        /// The content part of the call.
        index: usize,
        /// A fragment of the arguments' JSON.
        partial_json: String,
    },
    /// The end: the whole answer.
    Completed(ChatResponse),
}

/// The vectors of one embedding call, in the order of the inputs.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Embeddings {
    /// One vector per input.
    pub vectors: Vec<Vec<f32>>,
    /// What it cost.
    pub usage: Usage,
    /// `provider/model` that answered, stamped by the [`Router`](crate::Router).
    pub upstream: Option<String>,
}

impl Embeddings {
    /// Embeddings with their usage.
    pub fn new(vectors: Vec<Vec<f32>>, usage: Usage) -> Self {
        Self {
            vectors,
            usage,
            upstream: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_response_exposes_its_text_and_tool_calls() {
        let r = ChatResponse::new(
            "m",
            vec![
                Content::text("Hello, "),
                Content::ToolUse {
                    id: "t1".into(),
                    name: "lookup".into(),
                    input: json!({"id": 7}),
                },
                Content::text("world"),
            ],
            StopReason::ToolUse,
            Usage::default(),
        );
        assert_eq!(r.text(), "Hello, world");
        assert_eq!(r.tool_uses(), vec![("t1", "lookup", &json!({"id": 7}))]);
        assert_eq!(r.to_message().role, Role::Assistant);
    }

    #[test]
    fn usage_adds_up_and_counts_every_input_token() {
        let a = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_write_tokens: 1,
        };
        let mut b = a;
        b += a;
        assert_eq!(b.output_tokens, 10);
        assert_eq!(a.total_input(), 111);
    }

    #[test]
    fn content_serialises_with_a_type_tag() {
        let v = serde_json::to_value(Content::tool_result("t1", "ok", false)).unwrap();
        assert_eq!(
            v,
            json!({"type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false})
        );
    }
}
