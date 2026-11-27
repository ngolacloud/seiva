//! `seiva-agent` — the agent harness a developer extends rather than rewrites
//! (ADR 0048, phase G5).
//!
//! ```text
//! person ─▶ Agent::run ─▶ gateway (identity, budget, masking, guardrails, audit) ─▶ model
//!                │                                                                    │
//!                └──── tool calls ◀───────────────────────────────────────────────────┘
//!                      hooks → approver (writes) → Tool → result (labelled untrusted)
//! ```
//!
//! - **Bounded.** Steps, tokens, wall time and a [`Cancel`] signal ([`Limits`],
//!   OWASP LLM06/LLM10). A limit ends the run with its [`StopCause`], not an error.
//! - **Through the gateway.** Every model call is a `seiva-ai-gateway` call, so the
//!   caller's identity, budget, masking, guardrails and audit apply to each step.
//! - **Tools.** [`FnTool`] for a function, [`facades::tools`] for module facades
//!   in-process (their `#[requires]` checked as the person), [`mcp::tools`] for an
//!   allowlisted MCP server — its tool descriptions pinned by hash, a changed one
//!   withheld until approved.
//! - **Writes wait.** A write tool runs only if the [`Approver`] says so; the
//!   default, [`DenyWrites`], refuses every write.
//! - **Extensible.** [`Tool`], [`Hook`] (before and after each call), [`Approver`],
//!   [`Memory`] (kept per principal), and the gateway's own `Guardrail` and
//!   `Detector`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod agent;
pub mod mcp;
mod tool;

pub use agent::{
    Agent, AgentBuilder, AgentError, AgentRun, ApproveWrites, Approver, BuildError, CallOutcome,
    CallRecord, Cancel, DenyWrites, Hook, HookDecision, InMemoryMemory, Limits, Memory, StopCause,
};
#[cfg(feature = "facades")]
pub use tool::facades;
pub use tool::{AgentCtx, FnTool, Tool, ToolCall, ToolOutput};
