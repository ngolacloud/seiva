//! The traits: a provider (any model, by name), a model bound to a route, and an
//! embedder.

use async_trait::async_trait;

use crate::stream::EventStream;
use crate::{ChatRequest, ChatResponse, Content, Embeddings, LlmError, StreamEvent};

/// A model provider: one API (Anthropic, an OpenAI-compatible server…) that
/// serves many models. Adapters implement it; a [`Router`](crate::Router) holds
/// providers and picks one per route.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// The provider's name in logs and in [`ChatResponse::upstream`]
    /// (`anthropic`, `openai`, `gemini`…).
    fn name(&self) -> &str;

    /// Ask `model` for a whole answer.
    async fn complete(&self, model: &str, request: &ChatRequest) -> Result<ChatResponse, LlmError>;

    /// Ask `model` for a streamed answer. Errors before the first event (a bad
    /// status, a refused connection) are returned here, so a router can still fall
    /// back; errors after it arrive on the stream.
    ///
    /// The default asks for the whole answer and replays it as events, for a
    /// provider without streaming.
    async fn stream(&self, model: &str, request: &ChatRequest) -> Result<EventStream, LlmError> {
        let response = self.complete(model, request).await?;
        Ok(replay(response))
    }
}

/// A model reached by name — in practice a route of the
/// [`Router`](crate::Router). What a use case depends on: it never names a
/// provider or a provider's model id.
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// A whole answer.
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError>;

    /// A streamed answer.
    async fn stream(&self, request: &ChatRequest) -> Result<EventStream, LlmError>;
}

/// Turns text into vectors (retrieval, ADR 0048 §5).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// One vector per input, in order.
    async fn embed(&self, model: &str, inputs: &[String]) -> Result<Embeddings, LlmError>;
}

/// A whole answer as the events a streamed one would have produced.
pub fn replay(response: ChatResponse) -> EventStream {
    let mut events: Vec<Result<StreamEvent, LlmError>> = Vec::new();
    for (index, part) in response.content.iter().enumerate() {
        match part {
            Content::Text { text } if !text.is_empty() => events.push(Ok(StreamEvent::TextDelta {
                index,
                text: text.clone(),
            })),
            Content::ToolUse { id, name, input } => {
                events.push(Ok(StreamEvent::ToolUseStart {
                    index,
                    id: id.clone(),
                    name: name.clone(),
                }));
                events.push(Ok(StreamEvent::ToolInputDelta {
                    index,
                    partial_json: input.to_string(),
                }));
            }
            _ => {}
        }
    }
    events.push(Ok(StreamEvent::Completed(response)));
    Box::pin(futures_util::stream::iter(events))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StopReason, Usage};
    use futures_util::StreamExt;

    #[tokio::test]
    async fn a_replayed_answer_ends_with_the_whole_response() {
        let r = ChatResponse::new(
            "m",
            vec![Content::text("hi")],
            StopReason::EndTurn,
            Usage::default(),
        );
        let events: Vec<_> = replay(r.clone()).collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events.last().unwrap().as_ref().unwrap(),
            &StreamEvent::Completed(r)
        );
    }
}
