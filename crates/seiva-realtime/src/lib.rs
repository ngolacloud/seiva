//! Delonix Seiva — a first-class, protocol-neutral **WebSocket + SSE hub** (ADR
//! 0031): the framework's real-time primitive.
//!
//! Seiva's HTTP transport is strictly request/response. A platform, though, needs
//! the server to *push*: live collaboration, presence, dashboards that update in
//! place, notifications, and signalling for anything WebRTC. This crate is that
//! push channel, built once, correctly — so a product mounts its protocol as
//! policy instead of re-deriving the hub, the caps, and the security in every
//! service.
//!
//! # The shape
//!
//! - [`Hub`] — a sharded registry of live connections grouped by [`Topic`].
//!   [`publish`](Hub::publish) fans an opaque [`Frame`] out to every local
//!   subscriber and hands it to the bridge for other pods.
//! - [`Frame`] — the opaque payload. The hub carries **frames, not meanings**; a
//!   product's message type implements [`RealtimeMessage`] (encode/decode) and the
//!   hub never parses it. Signalling, chat and presence are the product's policy.
//! - [`authorize_upgrade`] — the deny-by-default gate: authenticate the
//!   connection-token through a [`RealtimeAuth`] and check the `Origin` against an
//!   [`OriginAllowlist`] **before** the upgrade completes, binding the socket to a
//!   [`Principal`](seiva_identity::Principal) + [`Scope`](seiva_ports::Scope).
//! - [`BroadcastBackend`] — the multi-pod seam (a Redis adapter behind the
//!   `redis-bridge` feature); [`NoBridge`] (local-only) is the default.
//!
//! # Zero-cost when unused, DORM-free
//!
//! The library links **no runtime** (`tokio` enters with no executor feature) and
//! no data layer. The axum WebSocket/SSE transport is behind the `ws` feature and
//! the Redis bridge behind `redis-bridge`; a service that pushes nothing enables
//! neither and pays for neither.
//!
//! # Safety model (non-negotiable, ADR 0031)
//!
//! - **Authenticate before upgrade** ([`authorize_upgrade`]) — deny by default.
//! - **`Origin` allowlist** ([`OriginAllowlist`]) — stops cross-site WebSocket
//!   hijacking (CSWSH).
//! - **Bounded send buffer** per connection — a slow consumer is *shed*, never
//!   buffered without bound ([`Hub::publish`] reports it in [`Delivery::shed`]).
//! - **Per-process and per-tenant caps** — a persistent connection is a different
//!   resource than a request and is bounded over its whole life ([`HubConfig`]).

#![forbid(unsafe_code)]

mod auth;
mod bridge;
mod frame;
mod hub;

#[cfg(feature = "ws")]
mod ws;

#[cfg(feature = "redis-bridge")]
mod redis_bridge;

pub use auth::{
    authorize_upgrade, Authenticated, OriginAllowlist, RealtimeAuth, StaticRealtimeAuth,
};
pub use bridge::{BridgeError, BroadcastBackend, NoBridge};
pub use frame::{ConnectionId, Frame, RealtimeMessage, Topic};
pub use hub::{ConnectionSink, Delivery, Hub, HubConfig};

#[cfg(feature = "ws")]
pub use ws::{serve_sse, serve_ws, Inbound, WsConfig};

#[cfg(feature = "redis-bridge")]
pub use redis_bridge::RedisBridge;

/// Why a real-time upgrade or publish was refused.
///
/// Free of any HTTP type so the hub stays transport-neutral; the `ws` transport
/// maps each variant to a status (`401`/`403`/`503`/`413`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RealtimeError {
    /// No valid connection-token was presented: missing, empty, or unknown. The
    /// deny-by-default answer (maps to `401`).
    Unauthenticated,
    /// The request's `Origin` is not in the allowlist — a likely CSWSH attempt
    /// (maps to `403`).
    ForbiddenOrigin,
    /// The per-process connection cap is reached; the upgrade is refused (maps to
    /// `503`, the long-lived analogue of ADR 0026's request shedding).
    TooManyConnections,
    /// The caller's tenant is at its per-tenant connection cap (maps to `503`).
    TenantQuotaExceeded,
    /// A frame exceeded `max_frame_size` (maps to `413`).
    FrameTooLarge {
        /// The offending frame's size in bytes.
        size: usize,
        /// The configured maximum in bytes.
        max: usize,
    },
}

impl std::fmt::Display for RealtimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RealtimeError::Unauthenticated => f.write_str("unauthenticated real-time connection"),
            RealtimeError::ForbiddenOrigin => f.write_str("origin not allowed (CSWSH guard)"),
            RealtimeError::TooManyConnections => f.write_str("too many connections"),
            RealtimeError::TenantQuotaExceeded => f.write_str("tenant connection quota exceeded"),
            RealtimeError::FrameTooLarge { size, max } => {
                write!(f, "frame too large: {size} bytes exceeds max {max}")
            }
        }
    }
}

impl std::error::Error for RealtimeError {}
