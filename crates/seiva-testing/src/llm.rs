//! Doubles for the model port (`seiva-llm`, ADR 0048): a provider that plays a
//! script and records what it was asked, and a deterministic embedder.
//!
//! ```
//! use std::sync::Arc;
//! use seiva_llm::{ChatModel, ChatRequest, Residency, Route, Router};
//! use seiva_testing::llm::ScriptedModel;
//! # async fn demo() {
//! let model = Arc::new(ScriptedModel::new("fake").reply_text("Olá!"));
//! let router = Router::builder()
//!     .provider(model.clone(), Residency::InCountry)
//!     .route(Route::new("chat", Residency::InCountry).public().upstream("fake", "m"))
//!     .build()
//!     .unwrap();
//! let answer = router.route("chat").unwrap().complete(&ChatRequest::new(64).user("hi")).await.unwrap();
//! assert_eq!(answer.text(), "Olá!");
//! assert_eq!(model.calls()[0].request.messages.len(), 1);
//! # }
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use seiva_llm::{
    ChatProvider, ChatRequest, ChatResponse, Content, Embedder, Embeddings, LlmError, StopReason,
    Usage,
};
use serde_json::Value;

/// One call a [`ScriptedModel`] received.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCall {
    /// The model id asked for.
    pub model: String,
    /// The request, exactly as the provider saw it.
    pub request: ChatRequest,
}

/// A [`ChatProvider`] that answers from a script, in order, and records every
/// request — so a test can assert both on what the use case did with the answer
/// and on what it sent (that a value was masked, say). A call after the script
/// runs out fails with [`LlmError::Config`], so an unexpected extra call is a
/// test failure, not a silent default. Streaming replays the scripted answer as
/// events. Clones share the script and the record.
#[derive(Clone)]
pub struct ScriptedModel {
    name: String,
    script: Arc<Mutex<VecDeque<Result<ChatResponse, LlmError>>>>,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl ScriptedModel {
    /// An empty script, registered under `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            script: Arc::default(),
            calls: Arc::default(),
        }
    }

    /// Then answer with this response.
    #[must_use]
    pub fn reply(self, response: ChatResponse) -> Self {
        self.script.lock().unwrap().push_back(Ok(response));
        self
    }

    /// Then answer with this text.
    #[must_use]
    pub fn reply_text(self, text: impl Into<String>) -> Self {
        let text = text.into();
        let usage = Usage {
            input_tokens: 10,
            output_tokens: text.split_whitespace().count() as u64,
            ..Usage::default()
        };
        let model = self.name.clone();
        self.reply(ChatResponse::new(
            model,
            vec![Content::text(text)],
            StopReason::EndTurn,
            usage,
        ))
    }

    /// Then ask to call `tool` with `input`.
    #[must_use]
    pub fn reply_tool_call(
        self,
        id: impl Into<String>,
        tool: impl Into<String>,
        input: Value,
    ) -> Self {
        let model = self.name.clone();
        self.reply(ChatResponse::new(
            model,
            vec![Content::ToolUse {
                id: id.into(),
                name: tool.into(),
                input,
            }],
            StopReason::ToolUse,
            Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Usage::default()
            },
        ))
    }

    /// Then fail with `error`.
    #[must_use]
    pub fn fail(self, error: LlmError) -> Self {
        self.script.lock().unwrap().push_back(Err(error));
        self
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }

    /// How many scripted answers are left.
    pub fn remaining(&self) -> usize {
        self.script.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for ScriptedModel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, model: &str, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.calls.lock().unwrap().push(RecordedCall {
            model: model.to_owned(),
            request: request.clone(),
        });
        self.script.lock().unwrap().pop_front().unwrap_or_else(|| {
            Err(LlmError::Config(format!(
                "the script of `{}` has run out",
                self.name
            )))
        })
    }
}

/// A deterministic [`Embedder`]: each text hashes (FNV-1a over its words) into a
/// unit vector of `dimensions` — the same text always gives the same vector, and
/// texts that share words point the same way. For retrieval tests, not meaning.
#[derive(Debug, Clone, Copy)]
pub struct HashEmbedder {
    dimensions: usize,
}

impl HashEmbedder {
    /// Vectors of `dimensions` components (at least 1).
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions: dimensions.max(1),
        }
    }

    fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dimensions];
        for word in text.split_whitespace() {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in word.to_lowercase().bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            v[(h % self.dimensions as u64) as usize] += 1.0;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    async fn embed(&self, _model: &str, inputs: &[String]) -> Result<Embeddings, LlmError> {
        let vectors = inputs.iter().map(|t| self.vector(t)).collect();
        let usage = Usage {
            input_tokens: inputs
                .iter()
                .map(|t| t.split_whitespace().count() as u64)
                .sum(),
            ..Usage::default()
        };
        Ok(Embeddings::new(vectors, usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_script_plays_in_order_and_records_each_request() {
        let m = ScriptedModel::new("fake")
            .reply_tool_call("t1", "lookup", json!({"id": 7}))
            .fail(LlmError::status(429, "slow down"))
            .reply_text("done");
        let req = ChatRequest::new(10).user("hi");
        assert_eq!(
            m.complete("a", &req).await.unwrap().stop_reason,
            StopReason::ToolUse
        );
        assert!(m.complete("a", &req).await.unwrap_err().is_retryable());
        assert_eq!(m.complete("b", &req).await.unwrap().text(), "done");
        assert!(matches!(
            m.complete("c", &req).await,
            Err(LlmError::Config(_))
        ));
        let calls = m.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[2].model, "b");
        assert_eq!(calls[0].request, req);
    }

    #[tokio::test]
    async fn the_hash_embedder_is_deterministic_and_unit_length() {
        let e = HashEmbedder::new(16);
        let a = e
            .embed("m", &["o cliente pediu um estorno".into()])
            .await
            .unwrap();
        let b = e
            .embed("m", &["o cliente pediu um estorno".into()])
            .await
            .unwrap();
        assert_eq!(a.vectors, b.vectors);
        let norm: f32 = a.vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
