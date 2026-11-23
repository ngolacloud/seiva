//! A durable, retrying **outbound-delivery engine** on top of the signing this
//! crate already ships (ADR 0033).
//!
//! `seiva-webhooks` signs a webhook correctly — the `HMAC-SHA256` over
//! `{id}.{timestamp}.{payload}`, the three headers, the replay window. That is
//! the hard *cryptographic* half and it is untouched here. What this module adds
//! is everything *around* the signature: taking a payload destined for a tenant's
//! endpoint and delivering it **reliably** — because a receiver is down, slow or
//! flaky far more often than it is healthy-and-instant.
//!
//! # The shape
//!
//! - A [`DeliveryLedger`] — the durable source of truth: one [`Delivery`] row per
//!   `(subscription, event)`, carrying its [`DeliveryStatus`]
//!   (`Pending` → `InFlight` → `Delivered` | `Failed` | `Dead`), the attempt
//!   count, `next_attempt_at`, and the last HTTP status + response excerpt. An
//!   [`InMemoryLedger`] double runs it offline in dev and test; a product binds a
//!   DORM-backed row for real durability.
//! - An [`HttpSender`] port — the one seam that actually touches the network, so
//!   the engine (and this crate) link no HTTP client. A test binds a scripted
//!   double; a product binds a `reqwest`/`hyper` adapter.
//! - A [`SigningSecrets`] resolver — hands the engine the [`Webhook`](crate::Webhook)
//!   signer for a subscription. The secret lives here, **never in the ledger**.
//! - The [`DeliveryEngine`] — claims due rows under a lease, signs with
//!   `seiva-webhooks`, validates egress, POSTs under a bounded timeout and a
//!   **per-endpoint circuit breaker**, and records the outcome: a transient
//!   failure reschedules with exponential backoff (reusing
//!   [`seiva_resilience::RetryPolicy`]'s schedule) until a cap, past which the row
//!   is **dead-lettered**.
//!
//! # Guarantees
//!
//! - **At-least-once with idempotency.** A retried delivery may arrive twice; it
//!   carries the same [`webhook-id`](crate::ID_HEADER) every time so the receiver
//!   deduplicates. The engine never pretends exactly-once over the network.
//! - **Durable, off the request path.** A delivery is *recorded as owed* and sent
//!   asynchronously by a worker `tick`; a slow receiver never holds an API request
//!   open, and a crash loses nothing the ledger committed.
//! - **Per-endpoint isolation.** One tenant's dead endpoint trips its own breaker
//!   and its rows are *deferred*, so it cannot starve delivery to everyone else.
//! - **Egress-safe.** Every destination is [validated](validate_egress) against
//!   loopback / link-local / RFC 1918 / cloud-metadata ranges before the POST — a
//!   conservative in-engine SSRF guard until ADR 0045 makes it a shared primitive.
//!
//! # Runtime-free library
//!
//! The engine composes futures; it starts no runtime of its own. A worker host
//! (the job queue of ADR 0034, or `seiva-worker`) calls [`DeliveryEngine::tick`]
//! on a cadence; the timing of the retry is the stored `next_attempt_at`, not an
//! in-process sleep. The only timer is the per-POST [`seiva_resilience::timeout`],
//! driven by the caller's runtime.

mod egress;
mod engine;
mod ledger;
mod memory;

pub use egress::{validate_egress, EgressError};
pub use engine::{
    DeliveryConfig, DeliveryEngine, DeliveryError, HttpRequest, HttpResponse, HttpSender,
    SendError, SigningSecrets, TickReport,
};
pub use ledger::{
    Delivery, DeliveryId, DeliveryLedger, DeliveryStatus, DeliveryUpdate, LedgerError,
    LedgerResult, NewDelivery,
};
pub use memory::InMemoryLedger;
