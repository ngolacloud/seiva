//! The delivery **ledger**: the durable record of what is owed and what happened.
//!
//! One [`Delivery`] row per `(subscription, event)`. Its [`DeliveryStatus`] is the
//! whole lifecycle; the [`DeliveryLedger`] port is the two operations the engine
//! needs on top of enqueue — *claim the due rows* and *record an outcome* — kept
//! abstract so the same engine runs over the in-memory double or a DORM-backed
//! table without change.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use seiva_ports::Scope;

/// Where a delivery is in its lifecycle.
///
/// The happy path is `Pending → InFlight → Delivered`. The two non-delivered
/// terminal states are distinct on purpose, because they call for different
/// operator responses:
///
/// - [`Failed`](Self::Failed) — a **permanent**, non-retryable rejection: a
///   blocked egress target, an unknown subscription (no signer), or a client
///   HTTP status the receiver will never change its mind on (a `400`/`404`). The
///   payload is wrong *as sent*; retrying it is pointless.
/// - [`Dead`](Self::Dead) — the **retry budget is spent**: the attempts were all
///   transient (a timeout, a `5xx`, a connection error) and the max-attempt cap
///   was reached. The payload may well succeed later; it is dead-lettered for
///   inspection and manual replay rather than retried forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// Owed and due for an attempt once `next_attempt_at` has arrived — either a
    /// first attempt or a scheduled retry after a transient failure.
    Pending,
    /// Claimed by a worker under a lease; an attempt is in progress. A row stuck
    /// here past its lease is recovered by a later [`claim_due`](DeliveryLedger::claim_due).
    InFlight,
    /// A `2xx` was received. Terminal success.
    Delivered,
    /// A permanent, non-retryable failure. Terminal; see the type docs.
    Failed,
    /// The transient-retry budget was exhausted. Terminal dead-letter; see the
    /// type docs.
    Dead,
}

impl DeliveryStatus {
    /// The lowercase token for a JSON body, a metric label or a log field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryStatus::Pending => "pending",
            DeliveryStatus::InFlight => "in_flight",
            DeliveryStatus::Delivered => "delivered",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Dead => "dead",
        }
    }

    /// Whether this is a terminal state — no further attempt will be made.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            DeliveryStatus::Delivered | DeliveryStatus::Failed | DeliveryStatus::Dead
        )
    }
}

/// A delivery's stable identifier — it **is** the [`webhook-id`](crate::ID_HEADER)
/// that travels on the wire, and therefore the receiver's idempotency key. The
/// same id on every redelivered attempt is what lets an at-least-once receiver
/// deduplicate (ADR 0011).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeliveryId(String);

impl DeliveryId {
    /// Wrap an id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as it goes in the [`webhook-id`](crate::ID_HEADER) header.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A delivery to enqueue: everything needed to make and retry the call, minus the
/// ledger bookkeeping (status, attempts, timestamps) the ledger assigns.
///
/// Note it carries **no signing secret** — the secret is resolved at send time by
/// [`SigningSecrets`](super::SigningSecrets), so it never lands in the durable
/// record.
#[derive(Debug, Clone)]
pub struct NewDelivery {
    /// The stable id = the `webhook-id` = the receiver's idempotency key.
    pub id: DeliveryId,
    /// Which subscription this delivery is for (selects the signer and endpoint
    /// policy; the engine resolves the signer from it).
    pub subscription: String,
    /// The domain event that triggered this delivery (for audit/dedup).
    pub event_id: String,
    /// The destination URL. Egress-validated before every POST.
    pub endpoint: String,
    /// The raw body, signed and POSTed byte for byte.
    pub payload: Vec<u8>,
    /// The ambient tenant scope this delivery belongs to. `None` fails closed at
    /// enqueue (ADR 0019): a delivery with no tenant is a bug, not a shared-root
    /// send.
    pub scope: Option<Scope>,
}

/// A durable delivery row: a [`NewDelivery`] plus the engine's bookkeeping.
#[derive(Debug, Clone)]
pub struct Delivery {
    /// The stable id / `webhook-id` / idempotency key.
    pub id: DeliveryId,
    /// The subscription selecting the signer and endpoint.
    pub subscription: String,
    /// The triggering domain event id.
    pub event_id: String,
    /// The destination URL.
    pub endpoint: String,
    /// The raw body to sign and POST.
    pub payload: Vec<u8>,
    /// The owning tenant scope.
    pub scope: Scope,
    /// Where the delivery is in its lifecycle.
    pub status: DeliveryStatus,
    /// How many attempts have been **made** so far (`0` before the first).
    pub attempt: u32,
    /// When the next attempt becomes due. A [`Pending`](DeliveryStatus::Pending)
    /// row is claimable once this is `<= now`.
    pub next_attempt_at: SystemTime,
    /// The HTTP status of the most recent attempt that got a response, if any.
    pub last_http_status: Option<u16>,
    /// A short, truncated excerpt of the most recent response or failure reason —
    /// for debugging. A secret or signature is never recorded here.
    pub last_response_excerpt: Option<String>,
}

/// The mutation [`DeliveryLedger::record`] applies after an attempt: the new
/// status, the attempt counter, when (if ever) to try again, and what the
/// receiver said.
#[derive(Debug, Clone)]
pub struct DeliveryUpdate {
    /// The row's new status.
    pub status: DeliveryStatus,
    /// The attempt count after this attempt.
    pub attempt: u32,
    /// When the next attempt is due (ignored once `status` is terminal).
    pub next_attempt_at: SystemTime,
    /// The HTTP status observed, if a response came back.
    pub last_http_status: Option<u16>,
    /// A short excerpt of the response / reason.
    pub last_response_excerpt: Option<String>,
}

/// Everything that can go wrong at the ledger boundary.
///
/// Small unit/one-field variants so a `Result` returning it stays well under
/// clippy's `result_large_err` threshold.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    /// A delivery was enqueued or claimed with no tenant [`Scope`] — fail-closed
    /// (ADR 0019) rather than record a tenant-less row.
    #[error("delivery has no tenant scope (fail-closed)")]
    MissingScope,
    /// [`record`](DeliveryLedger::record) named an id the ledger does not hold.
    #[error("no such delivery in the ledger")]
    NotFound,
    /// The backing store failed (a DB error for a real adapter). The message is
    /// safe to log — it carries no payload or secret.
    #[error("delivery ledger backend error: {0}")]
    Backend(String),
}

/// A ledger operation's result.
pub type LedgerResult<T> = Result<T, LedgerError>;

/// The durable store of deliveries — the source of truth the engine drives.
///
/// Three operations, deliberately minimal:
///
/// - [`enqueue`](Self::enqueue) records a new owed delivery (idempotent on id, so
///   the transactional "one row per matching subscription" write of ADR 0021 can
///   be replayed safely).
/// - [`claim_due`](Self::claim_due) atomically leases the due rows to one worker —
///   the lease is what makes delivery safe at N replicas.
/// - [`record`](Self::record) writes an attempt's outcome back.
///
/// `get` is for inspection / replay (and the tests). An adapter backs this with a
/// row in the data layer; the [`InMemoryLedger`](super::InMemoryLedger) backs it
/// with a map for dev and test.
#[async_trait]
pub trait DeliveryLedger: Send + Sync {
    /// Record a new owed delivery. **Idempotent on [`DeliveryId`]**: enqueuing the
    /// same id twice leaves a single row, so a replayed outbox drain never
    /// duplicates a delivery. Fails closed on a `None` scope.
    async fn enqueue(&self, new: NewDelivery) -> LedgerResult<()>;

    /// Atomically claim up to `max` deliveries that are **due** — every
    /// [`Pending`](DeliveryStatus::Pending) row whose `next_attempt_at <= now`,
    /// plus any [`InFlight`](DeliveryStatus::InFlight) row whose lease has expired
    /// (a crashed worker's orphan) — marking each `InFlight` under a lease of
    /// `lease` from `now`. The returned rows are this worker's to deliver; another
    /// worker's concurrent `claim_due` will not see them until the lease lapses.
    async fn claim_due(
        &self,
        now: SystemTime,
        max: usize,
        lease: Duration,
    ) -> LedgerResult<Vec<Delivery>>;

    /// Apply an attempt's [`DeliveryUpdate`] to the row `id`.
    async fn record(&self, id: &DeliveryId, update: DeliveryUpdate) -> LedgerResult<()>;

    /// Fetch a row by id, for inspection, dead-letter replay or assertions.
    async fn get(&self, id: &DeliveryId) -> LedgerResult<Option<Delivery>>;
}
