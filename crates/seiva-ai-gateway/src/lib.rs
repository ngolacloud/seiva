//! `seiva-ai-gateway` — the controlled path between a product and any model
//! (ADR 0048, phase G2).
//!
//! Every model call goes through the same stages, in this order, each one failing
//! closed:
//!
//! | Stage | What it does | OWASP LLM Top 10 |
//! |---|---|---|
//! | Identify | an authenticated [`Principal`](seiva_identity::Principal) is required | — |
//! | Authorize | the route's permission, the caller's residency, every tool offered | LLM06 |
//! | Budget | tokens held per principal, tenant and area; a store outage stops the call | LLM10 |
//! | Mask | personal data becomes `‹LABEL_N›`; a credential refuses the request; tool output is labelled untrusted | LLM01, LLM02, LLM07 |
//! | Guard (in) | the product's guardrails on the masked request | LLM01 |
//! | model | the [`seiva_llm`] route | — |
//! | Guard (out) | no credential in the answer; no tool the model was not offered; the product's guardrails | LLM02, LLM05, LLM06 |
//! | Unmask | placeholders restored for a caller entitled to them | LLM02 |
//! | Settle | the real usage counted | LLM10 |
//! | Audit | one record per call — refusals too — with counts and names, never values | — |
//!
//! ```
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! use seiva_ai_gateway::{BudgetScope, Caller, Gateway, InMemoryBudget};
//! use seiva_identity::Principal;
//! use seiva_llm::{ChatRequest, Residency, Route, Router};
//! use seiva_testing::llm::ScriptedModel;
//! use seiva_testing::{FixedClock, RecordingAuditSink};
//! # async fn demo() {
//! let model = Arc::new(ScriptedModel::new("cloud").reply_text("Recebido, ‹TAX_ID_1›."));
//! let router = Router::builder()
//!     .provider(model.clone(), Residency::Abroad)
//!     .route(Route::new("support-chat", Residency::Abroad).requires("llm.support").upstream("cloud", "m"))
//!     .build()
//!     .unwrap();
//! let budget = InMemoryBudget::new(Arc::new(FixedClock::at_unix_secs(0)), Duration::from_secs(86_400))
//!     .per(BudgetScope::Principal, 200_000);
//! let gateway = Gateway::builder(router)
//!     .budget(Arc::new(budget))
//!     .audit(Arc::new(RecordingAuditSink::new()))
//!     .build()
//!     .unwrap();
//!
//! let ana = Caller::new(Principal::new("ana", ["llm.support"]));
//! let gated = gateway
//!     .complete(&ana, "support-chat", ChatRequest::new(256).user("O meu NIF 5417123456 está certo?"))
//!     .await
//!     .unwrap();
//! // The model never saw the NIF; Ana sees it back.
//! let sent = &model.calls()[0].request.messages[0].content[0];
//! assert_eq!(sent, &seiva_llm::Content::text("O meu NIF ‹TAX_ID_1› está certo?"));
//! assert_eq!(gated.response.text(), "Recebido, 5417123456.");
//! # }
//! # tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(demo());
//! ```
//!
//! In-process only in this phase; the server shape (OpenAI- and
//! Anthropic-compatible endpoints) is G3.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod budget;
pub mod detect;
mod error;
mod gateway;
mod guard;
mod mask;

pub use budget::{BudgetError, BudgetKey, BudgetScope, BudgetStore, InMemoryBudget, Reservation};
pub use detect::{Detector, PatternDetector};
pub use error::GatewayError;
pub use gateway::{
    AuditMode, BuildError, CallReport, Caller, CredentialPolicy, Gated, Gateway, GatewayBuilder,
    RevealPolicy, Stage,
};
pub use guard::{GuardContext, Guardrail, JsonOutput, SizeLimit, Verdict, UNTRUSTED_NOTICE};
pub use mask::{unmask_response, CredentialFound, Masker, Vault};
