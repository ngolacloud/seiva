//! `seiva-mcp-server` — module facades as MCP tools, on behalf of a person
//! (ADR 0048, phase G4).
//!
//! A facade method opts in to being a tool; nothing is exposed by default:
//!
//! ```ignore
//! #[seiva::facade(version = "v1")]
//! pub trait Payments {
//!     /// Refund a captured payment, fully or partly.
//!     #[requires("payments.refund")]
//!     #[tool(effect = "write")]
//!     async fn refund(&self, ctx: &Ctx, cmd: RefundPayment) -> Result<RefundDto, PaymentsError>;
//! }
//!
//! #[seiva::tools]                     // next to #[seiva::controller]
//! impl Payments for PaymentsService { /* … */ }
//!
//! let tools = ToolSet::new().with(payments_tools(service))?;
//! let mcp = McpServer::builder(tools, authenticator, audit_sink).resource("https://api.example.com/mcp").build();
//! let app = mcp.router();             // POST /mcp, /mcp/approvals/{id}, /.well-known/oauth-protected-resource
//! ```
//!
//! - **The contract is the source.** The tool name (`payments_refund`), its
//!   description (the doc comment), its input schema (the parameter's
//!   `WireSchema`), its permission (`#[requires]`) and its effect come from the
//!   facade descriptor, like the HTTP routes, the gRPC service and OpenAPI.
//! - **On behalf of the person.** The caller is the person whose token the MCP
//!   client presents, checked by the same `Authenticator` as the HTTP API; each
//!   tool's `#[requires]` is enforced here, and the method runs with that person's
//!   `Ctx` — so the BOLA scope (ADR 0005) applies exactly as over HTTP. A tool the
//!   caller may not use is neither listed nor callable (reported as unknown).
//! - **Writes wait for a person.** A write tool's first call returns an approval
//!   request; the app (the MCP host — not the model) shows it to the person
//!   (`GET /mcp/approvals/{id}`) and confirms it with their token (`POST`); the
//!   model's next call with the same arguments and `"_approval": id` runs it, once.
//!   `auto_approve(tool, permission)` is the audited exception.
//! - **Audited.** Every call, approval and refusal goes to the `AuditSink`; a
//!   write is recorded **before** it runs, and does not run if that fails (the
//!   critical audit of ADR 0043). Arguments are recorded as a digest, never in
//!   clear.
//!
//! The transport is MCP's Streamable HTTP with JSON responses (protocol
//! [`PROTOCOL_VERSION`]); the server opens no stream of its own, keeps no session
//! and implements `initialize`, `ping`, `tools/list` and `tools/call`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod server;
mod tools;

pub use server::{McpServer, McpServerBuilder, APPROVAL_ARG, PROTOCOL_VERSION};
pub use tools::{FacadeTools, Tool, ToolFailure, ToolFuture, ToolSet, ToolSetError};

/// What `#[seiva::tools]` expands to. Not a public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::tools::ToolFuture;
    pub use seiva_http::Ctx;
    pub use serde_json::Value;

    use crate::tools::ToolFailure;

    /// Decode a tool's arguments into the method's parameter.
    pub fn from_args<T: serde::de::DeserializeOwned>(
        args: &Value,
        descriptor: &seiva::contract::FacadeDescriptor,
        method: &str,
    ) -> Result<T, ToolFailure> {
        crate::tools::from_args(args, descriptor, method)
    }

    /// Encode a method's result.
    pub fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ToolFailure> {
        crate::tools::to_value(value)
    }

    /// A method's error, as the problem it maps to.
    pub fn failed<E: Into<seiva_contract::Problem>>(error: E) -> ToolFailure {
        crate::tools::failed(error)
    }

    /// No dispatch arm for this method.
    pub fn unknown(method: &str) -> ToolFailure {
        crate::tools::unknown(method)
    }
}
