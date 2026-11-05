//! Trait-only ports the application layer codes against.
//!
//! A module's `application` crate depends on this crate — never on
//! `delonix-seiva` — to declare the outbound capabilities a use case needs.
//! The `infrastructure` crate wires concrete implementations (DORM-backed or
//! external); the framework resolves them at composition time. Keeping this
//! crate free of any runtime, macro or data layer is what lets the application
//! layer stay independent of both the framework and the ORM.
//!
//! This is the first cut of the boundary; its exact shape is fixed by the Seiva
//! core ADR. The ports below are the ones a module relies on:
//! [`UnitOfWork`], [`EventPublisher`], [`Clock`], [`IdGenerator`] and the durable
//! [`IdempotencyStore`] (ADR 0011).

#![forbid(unsafe_code)]

use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

/// The error any port may surface, kept opaque and dependency-free: a boxed
/// std error so this crate carries no error library. Adapters map their
/// concrete errors (a DB error, a broker error) into this at the boundary, so
/// the application layer sees one uniform failure type.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result returned by every port.
pub type PortResult<T> = Result<T, BoxError>;

/// A 128-bit opaque identifier. Rendered as a UUID or ULID by the framework and
/// by adapters; this crate stays free of a UUID dependency so the boundary
/// imposes no identifier library on downstream code.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(pub [u8; 16]);

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({})", hex16(&self.0))
    }
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Reads the current time. Injected so use cases are deterministic under test:
/// a fake clock returns a fixed instant with no real time involved.
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Mints new identifiers. Injected so a test double can hand out a predictable
/// sequence instead of random values.
pub trait IdGenerator: Send + Sync {
    fn new_id(&self) -> Id;
}

/// Publishes a domain event. The payload is already serialized by the caller
/// (the interface layer owns the envelope format, e.g. CloudEvents); this port
/// only moves bytes to a topic, so no serialization library rides in here.
///
/// Whether the publish is transactional (outbox in the same transaction) or
/// best-effort is a property of the implementation and of `seiva.toml`, not of
/// this trait: the application layer asks to publish and the boundary decides
/// how the guarantee is kept.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()>;
}

/// The transaction boundary — **scope-based** (ADR 0002, amended by ADR 0004 of
/// the data-layer gateway).
///
/// The application decides where a transaction begins and ends, one per use
/// case, by running the use-case body inside [`transaction`](UnitOfWork::transaction):
/// the data layer **commits if the body returns `Ok` and rolls back if it
/// returns `Err`**. Statements the body performs through the data layer join
/// this transaction. To publish an event only after a successful commit (the
/// outbox-relay wake), act on the `Ok` the call returns — no separate
/// `on_commit` hook is needed.
///
/// Scope-based rather than handle-based (`begin`/`commit`) because the DORM
/// executor is closure-scoped (`db::atomic`): a `commit()` called later could
/// not drive a closure that had already returned. Used generically (a handler is
/// generic over its ports), so this trait need not be object-safe.
pub trait UnitOfWork: Send + Sync {
    /// Runs `work` in a transaction, committing on `Ok` and rolling back on `Err`.
    ///
    /// Returns a **boxed `Send` future** (`Pin<Box<dyn Future + Send>>`). The
    /// `Send` bound is part of the contract, so a use case that awaits it stays
    /// `Send` in a generic async context — an HTTP handler drives it directly,
    /// with no `spawn_blocking` offload. Boxing (one allocation per transaction,
    /// negligible against a database round-trip) erases the opaque future type,
    /// which also sidesteps the higher-ranked-lifetime inference limits an
    /// `impl Future` return hits with the generic `work` closure.
    fn transaction<'a, T, F, Fut>(
        &'a self,
        work: F,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = PortResult<T>> + Send + 'a,
        T: Send + 'a;
}

/// What claiming an idempotency key found — the outcome of [`IdempotencyStore::begin`].
///
/// The key is already **scoped** by the caller (tenant + identity + operation +
/// version folded in) before it reaches the store, so two tenants' keys can never
/// collide here. The `fingerprint` the caller passes alongside pins the key to one
/// request's semantically-relevant bytes; reusing the key with a different
/// fingerprint is [`Mismatch`](IdempotencyOutcome::Mismatch) and must not execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyOutcome {
    /// First claim: the caller performs the effect, then calls
    /// [`finish`](IdempotencyStore::finish) with the response to persist.
    Fresh,
    /// Already completed: the stored response bytes — return them, perform **no**
    /// effect. (The caller owns the encoding; the store moves opaque bytes.)
    Replay(Vec<u8>),
    /// A concurrent claim of the same key is still in flight: the caller waits a
    /// bounded time or answers a conflict (`409`/`425`), never executing twice.
    InProgress,
    /// The key was reused with a **different** fingerprint — the same key must
    /// mean the same operation. The caller answers `409` and does not execute.
    Mismatch,
}

/// A durable, distributed idempotency store — the reusable guarantee behind a
/// retried command, HTTP call, job or admin action (ADR 0011).
///
/// The base framework codes against this port and never against a specific
/// database: an adapter (the DORM gateway, say) provides the durability and the
/// cross-replica atomicity. The contract:
///
/// - [`begin`](Self::begin) atomically claims `scoped_key` for a request
///   fingerprinted `fingerprint`, valid for `ttl`. It is the only arbiter of
///   concurrency — a durable uniqueness/CAS, not an in-memory lock — so two
///   replicas racing the same key resolve to exactly one [`Fresh`](IdempotencyOutcome::Fresh).
/// - The caller performs the effect only on `Fresh`, then calls
///   [`finish`](Self::finish) with the response. For true atomicity the
///   reservation, the domain mutation and the outbox write run in **one**
///   transaction, so a committed key always has its effect and vice versa.
/// - [`finish`](Self::finish) persists the response so a later `begin` replays it
///   without re-executing.
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Claims `scoped_key` for `fingerprint`, valid `ttl`. See [`IdempotencyOutcome`].
    async fn begin(
        &self,
        scoped_key: &str,
        fingerprint: &str,
        ttl: Duration,
    ) -> PortResult<IdempotencyOutcome>;

    /// Persists `response` for `scoped_key` and marks it completed, so a later
    /// [`begin`](Self::begin) returns [`Replay`](IdempotencyOutcome::Replay).
    async fn finish(&self, scoped_key: &str, response: &[u8]) -> PortResult<()>;

    /// Deletes records whose TTL has elapsed; returns how many. A scheduled
    /// worker calls this so the store does not grow without bound. The default is
    /// a no-op (returns `0`) for stores that expire records some other way.
    async fn purge_expired(&self) -> PortResult<u64> {
        Ok(0)
    }
}

/// The store key for **consumer-side inbox deduplication**: processing one event
/// at most once per consumer (ADR 0011). Compose it from the consumer's name and
/// the event id, then claim it through [`IdempotencyStore::begin`] **in the same
/// transaction** as the effect — on [`Fresh`](IdempotencyOutcome::Fresh) apply
/// the effect and [`finish`](IdempotencyStore::finish); on any other outcome the
/// event was already handled, so skip it (and still ack after commit). A
/// redelivery of the same event id resolves to the same key, so the effect never
/// repeats even though the broker may deliver more than once.
///
/// ```
/// use seiva_ports::inbox_key;
/// assert_eq!(inbox_key("billing", "evt-123"), "inbox:billing:evt-123");
/// ```
#[must_use]
pub fn inbox_key(consumer: &str, event_id: &str) -> String {
    format!("inbox:{consumer}:{event_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_renders_as_lowercase_hex() {
        let bytes = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let id = Id(bytes);
        // Validate our dependency-free formatter against std's lowercase hex.
        let expected: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(format!("{id:?}"), format!("Id({expected})"));
    }
}
