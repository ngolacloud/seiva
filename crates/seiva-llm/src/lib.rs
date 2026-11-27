//! `seiva-llm` — the provider-neutral language-model port of Delonix Seiva
//! (ADR 0048, phase G1).
//!
//! A product never names a provider: it calls a **route** (`support-chat`,
//! `code-review`) through a [`ChatModel`]. The [`Router`] maps each route to its
//! upstreams — provider and model, in fallback order — and keeps the promise the
//! route makes about where data goes ([`Residency`]).
//!
//! ```text
//!  use case ──ChatModel──▶ Router ──route──▶ ChatProvider (Anthropic, OpenAI-compatible…)
//!                                                   │
//!                                             HttpTransport  (injected: reqwest, the
//!                                                   │         egress guard of ADR 0045)
//!                                                   ▼
//!                                                the model
//! ```
//!
//! # What is here
//!
//! - **Types** — [`ChatRequest`], [`Message`], [`Content`] (text, image, tool
//!   use, tool result), [`ToolSpec`], [`ChatResponse`], [`Usage`],
//!   [`StreamEvent`].
//! - **Traits** — [`ChatProvider`] (adapters), [`ChatModel`] (what a use case
//!   holds), [`Embedder`].
//! - **Routes** — [`Router`], [`Route`], [`Residency`]: checked when built
//!   (unknown providers, an `in_country` route with an `abroad` upstream, a route
//!   with no access policy are refused), then retry, per-attempt timeouts and
//!   fallback at run time.
//! - **Credentials** — [`CredentialSource`] ([`EnvCredentials`] by default) and
//!   [`Secret`], which never prints its value.
//! - **Codecs** — [`providers::anthropic::Anthropic`] and
//!   [`providers::openai::OpenAiCompatible`] (OpenAI, Azure, vLLM, Ollama,
//!   Gemini's compatible endpoint, an OpenAI-compatible gateway already on the
//!   network). They talk only through the [`HttpTransport`] port; this crate
//!   links no HTTP client.
//!
//! # A taste
//!
//! ```
//! # use std::sync::Arc;
//! use seiva_llm::{ChatModel, ChatRequest, Residency, Route, Router};
//! # use seiva_llm::{ChatProvider, ChatResponse, Content, LlmError, StopReason, Usage};
//! # struct Echo;
//! # #[async_trait::async_trait]
//! # impl ChatProvider for Echo {
//! #     fn name(&self) -> &str { "anthropic" }
//! #     async fn complete(&self, model: &str, _: &ChatRequest) -> Result<ChatResponse, LlmError> {
//! #         Ok(ChatResponse::new(model, vec![Content::text("Olá!")], StopReason::EndTurn, Usage::default()))
//! #     }
//! # }
//! # async fn demo() -> Result<(), LlmError> {
//! # let anthropic: Arc<dyn ChatProvider> = Arc::new(Echo);
//! let router = Router::builder()
//!     .provider(anthropic, Residency::Abroad)
//!     .route(
//!         Route::new("support-chat", Residency::Abroad)
//!             .requires("llm.support")
//!             .upstream("anthropic", "claude-sonnet-5-5"),
//!     )
//!     .build()?;
//!
//! let chat = router.route("support-chat")?;
//! let answer = chat.complete(&ChatRequest::new(512).user("Olá")).await?;
//! assert_eq!(answer.upstream.as_deref(), Some("anthropic/claude-sonnet-5-5"));
//! # Ok(()) }
//! # tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(demo()).unwrap();
//! ```
//!
//! # The gateway
//!
//! Identity, authorization, budget, masking, guardrails and audit are the
//! gateway stages of ADR 0048 §2, in the `seiva-ai-gateway` crate. This crate
//! gives them the route's permission and residency to check, and the usage to
//! meter; a product that calls a model on behalf of people goes through the
//! gateway, not the router.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod credentials;
mod error;
mod model;
pub mod providers;
mod router;
mod stream;
mod transport;
mod types;

pub use credentials::{CredentialSource, EnvCredentials, Secret, StaticCredentials};
pub use error::LlmError;
pub use model::{replay, ChatModel, ChatProvider, Embedder};
pub use router::{Residency, Route, RouteHandle, Router, RouterBuilder, Upstream};
/// The retry schedule of each upstream ([`RouterBuilder::retry`]), re-exported
/// from `seiva-resilience` so a composition root needs no extra dependency.
pub use seiva_resilience::RetryPolicy;
pub use stream::EventStream;
pub use transport::{
    BodyStream, Endpoint, HeaderValue, HttpRequest, HttpResponse, HttpTransport, Method,
    MAX_RESPONSE_BYTES,
};
pub use types::{
    ChatRequest, ChatResponse, Content, Embeddings, Message, Role, StopReason, StreamEvent,
    ToolChoice, ToolSpec, Usage,
};
