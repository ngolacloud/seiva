//! `seiva-mcp` — serve DORM's MCP server over HTTP behind Seiva's authentication
//! and zero-trust (first cut).
//!
//! # The boundary
//!
//! DORM keeps the MCP `stdio` transport and owns the MCP **protocol**: the
//! JSON-RPC request handler is [`dorm_mcp::handle_message`], which this crate
//! calls and never reimplements. The HTTP transport belongs to **Seiva**,
//! because "HTTP needs the authentication and zero trust of the framework". So
//! this crate is a fresh HTTP endpoint plus a Seiva auth layer that dispatches
//! into DORM's protocol core — it deliberately does **not** use `dorm-mcp`'s own
//! `serve_http` / `serve_on` or its OAuth. That is DORM's HTTP; Seiva replaces it
//! here with the framework's own authentication.
//!
//! ```text
//!   HTTP client ──POST /mcp──▶  Seiva auth (zero trust)  ──▶  dorm_mcp::handle_message
//!                              (this crate: transport+auth)      (DORM: protocol core)
//! ```
//!
//! # What this crate provides
//!
//! * [`auth`] — the authentication contract Seiva owns: an [`Authenticator`]
//!   turns a bearer token into a [`Principal`] and **denies by default**. The
//!   first-cut implementation is [`TokenAuthenticator`], a static token →
//!   principal map (optionally bridged from DORM's token spec via
//!   [`TokenAuthenticator::from_http_tokens`] / [`dorm_mcp::parse_tokens`]). Real
//!   OIDC / mTLS / OAuth token-exchange is a later step behind the same trait.
//! * [`http`] — the axum `POST /mcp` endpoint. It authenticates first, then
//!   builds a per-request [`ServerConfig`] whose `tenant` and `writable` come
//!   from the authenticated [`Principal`] (never a tool argument), and finally
//!   dispatches the request line through [`dorm_mcp::handle_message`]. The entry
//!   points are [`serve`] (run until Ctrl-C) and [`serve_with_shutdown`]
//!   (graceful shutdown on a provided signal); [`router`] exposes the bare
//!   [`axum::Router`] for embedding or testing.
//!
//! # Scope of the first cut
//!
//! The protocol core needs DORM's database, so a full `tools/call` request
//! cannot run without a live DB; the auth layer and the dispatch path for
//! DB-free methods (`initialize`, notifications) are fully exercised in tests.
//! DORM's own defence-in-depth read-only guard (`db::set_read_only`) is a core
//! global this crate does not drive; per-request `writable` is still narrowed to
//! the principal here, and wiring the global guard belongs with the DB-coupled
//! integration step.

#![forbid(unsafe_code)]

pub mod auth;
pub mod http;

pub use auth::{AuthError, Authenticator, Principal, TokenAuthenticator};
pub use http::{router, serve, serve_with_shutdown, AppState};

// Re-export the DORM protocol-config value types callers need to assemble a
// server, so a downstream need not also name the `dorm-mcp` git dependency.
pub use dorm_mcp::{parse_tokens, HttpToken, ServerConfig};
