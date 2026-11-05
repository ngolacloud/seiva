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

/// An audit record of a sensitive — typically administrative — mutation
/// (ADR 0013): who did what, to which object, with what result, and a
/// correlation id. Field-level `details` are **redacted** of secrets before they
/// ever reach a sink (see [`AuditEntry::redacted`]); the full request body with
/// its secrets is never stored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuditEntry {
    /// The acting principal's subject (empty for anonymous).
    pub actor: String,
    /// The tenant the action was scoped to, if any.
    pub tenant: Option<String>,
    /// The operation, e.g. `"POST /v1/accounts/create_user"`.
    pub operation: String,
    /// The target object's id, when the handler knows it.
    pub object: Option<String>,
    /// The result, e.g. the HTTP status as a string (`"201"`, `"403"`).
    pub outcome: String,
    /// The request's correlation id, for tracing.
    pub request_id: Option<String>,
    /// Redacted field-level detail (never the full body, never a secret value).
    pub details: Vec<(String, String)>,
}

impl AuditEntry {
    /// Returns the entry with every sensitive `details` value replaced by
    /// `"[redacted]"` ([`is_sensitive_field`]). Call this before recording so a
    /// password, hash, token or key can never land in the audit log.
    #[must_use]
    pub fn redacted(mut self) -> Self {
        for (name, value) in &mut self.details {
            if is_sensitive_field(name) {
                *value = "[redacted]".to_owned();
            }
        }
        self
    }
}

/// Whether a field name looks sensitive and so its value must be redacted from an
/// audit record or a log: passwords, hashes, tokens, keys, secrets, credentials
/// and authorization material (case-insensitive substring match — deliberately
/// broad, since over-redacting an audit detail is safe and under-redacting leaks).
#[must_use]
pub fn is_sensitive_field(name: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "hash",
        "apikey",
        "api_key",
        "credential",
        "authorization",
        "cookie",
        "private",
    ];
    let lower = name.to_ascii_lowercase();
    // A bare "key" is sensitive on its own, plus any of the needles as substrings.
    lower == "key" || NEEDLES.iter().any(|needle| lower.contains(needle))
}

/// A persistent sink for [`AuditEntry`] records (ADR 0013). The base framework
/// codes against this port; an adapter (the DORM gateway) persists the entries.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Persists one audit entry. Called with an already-[`redacted`](AuditEntry::redacted)
    /// entry.
    async fn record(&self, entry: &AuditEntry) -> PortResult<()>;
}

/// The outcome of a rate-limit check for one client (ADR 0015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitDecision {
    /// Within budget — serve the request.
    Permitted,
    /// Budget exhausted — reject (`429`); retry after this long.
    Limited {
        /// Time until the client's window rolls over.
        retry_after: Duration,
    },
}

/// A **distributed** rate-limit backend (ADR 0015): the authority a cluster
/// shares so a client's budget is enforced across replicas, not per-process.
///
/// The base framework's default limiter is in-process (fine for a single
/// instance, and documented as such); a deployment that runs several replicas
/// behind a load balancer wires a shared store (e.g. Redis) through this port so
/// one client cannot get `N × limit` by spreading requests across instances. The
/// check must be **atomic** in the backend (a single round trip that increments
/// and expires), not a read-modify-write, so concurrent replicas cannot race past
/// the limit. Rate limiting is abuse-dampening, not a durable business guarantee,
/// so a store that is briefly unavailable should fail **open** at the call site.
#[async_trait]
pub trait RateLimitStore: Send + Sync {
    /// Records one hit for `key` and reports whether it is within `limit` per
    /// `window`. Implementations increment-and-expire atomically.
    async fn check(&self, key: &str, limit: u32, window: Duration)
        -> PortResult<RateLimitDecision>;
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
    fn sensitive_fields_are_detected_case_insensitively() {
        for name in [
            "password", "Password", "api_key", "x-secret", "pwdHash", "key", "cookie",
        ] {
            assert!(is_sensitive_field(name), "{name} must be sensitive");
        }
        for name in ["username", "title", "amount", "keyboard_layout"] {
            assert!(!is_sensitive_field(name), "{name} must not be sensitive");
        }
    }

    #[test]
    fn redacted_masks_only_sensitive_values() {
        let entry = AuditEntry {
            operation: "POST /x".to_owned(),
            details: vec![
                ("username".to_owned(), "alice".to_owned()),
                ("password".to_owned(), "hunter2".to_owned()),
            ],
            ..AuditEntry::default()
        }
        .redacted();
        assert_eq!(
            entry.details[0],
            ("username".to_owned(), "alice".to_owned())
        );
        assert_eq!(
            entry.details[1],
            ("password".to_owned(), "[redacted]".to_owned())
        );
    }

    #[test]
    fn id_renders_as_lowercase_hex() {
        let bytes = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let id = Id(bytes);
        // Validate our dependency-free formatter against std's lowercase hex.
        let expected: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(format!("{id:?}"), format!("Id({expected})"));
    }
}
