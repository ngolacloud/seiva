//! The in-process event bus core (ADR 0021): modules choreograph through events
//! in a single process, with the same guarantees — and the same subscriber
//! code — as the broker path.
//!
//! Seiva's promise is that *modules cooperate through events, not calls*, and
//! that the same module runs as a modular monolith or as a microservice by
//! composition alone. Half of that already exists: a use case publishes into a
//! transactional outbox, and [`seiva-outbox-relay`] forwards the outbox to a
//! broker at-least-once, where the remote consumer deduplicates with an **inbox**
//! key (ADR 0011). This crate is the other half — the **in-process dispatcher**
//! that reads one [`CloudEvent`] and fans it out to locally-registered
//! subscribers, marking each delivery once through the same
//! [`inbox_key`] + [`IdempotencyStore`]. Because both paths read the same event
//! and dedup through the same inbox, a subscriber handler is **byte-identical**
//! across them, and delivery is at-least-once with exactly-one *effect* either
//! way. It is the in-process twin of `seiva_outbox_relay::drain`.
//!
//! ## What is here and what stays DORM-coupled
//!
//! This crate is **DORM-free and broker-free**, so the whole choreography is
//! testable fully offline. It provides:
//!
//! - [`EventSubscriber`] — the handler a module plugs in for an event type;
//! - [`SubscriberRegistry`] — the composition-root routing table, by type;
//! - [`dispatch`] — deliver one event to its subscribers, deduped per subscriber;
//! - [`OutboxReader`] — the DORM-free read seam over an outbox (read a batch,
//!   advance the cursor), the twin of what `dorm-outbox` does in-database;
//! - [`drain_in_process`] — the **driver**: one pass that reads a batch, dispatches
//!   each event, and advances the cursor only after a successful dispatch — the
//!   in-process twin of `seiva_outbox_relay::drain`;
//! - [`InMemoryOutbox`] (behind the `testing` feature) — an in-memory
//!   [`OutboxReader`] so the `orders.placed → inventory.reserved` choreography is
//!   proved with no broker and no database (see `tests/choreography.rs`).
//!
//! What stays out is only the **DORM adapter** of [`OutboxReader`] over the real
//! in-database outbox (`dorm-outbox`): it is the piece that makes `module split` a
//! swap of this driver for `seiva-outbox-relay`, and it lands there (that crate is
//! excluded from the default workspace because it is DORM-git-coupled). In the
//! integrated driver the `IdempotencyStore` is the DORM-backed one and the claim +
//! effect + mark share one transaction; here any `IdempotencyStore` double proves
//! the same logic.
//!
//! [`seiva-outbox-relay`]: https://github.com/ngolacloud/seiva
//! [`CloudEvent`]: seiva_messaging::CloudEvent
//! [`inbox_key`]: seiva_ports::inbox_key
//! [`IdempotencyStore`]: seiva_ports::IdempotencyStore

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use delonix_seiva::worker::run_worker;
use seiva_messaging::CloudEvent;
use seiva_ports::{inbox_key, IdempotencyOutcome, IdempotencyStore, PortResult, Scope, UnitOfWork};

// The worker runtime (interval loop + graceful shutdown + metrics) is the
// framework's; this crate only supplies the per-pass job. Re-exported so a caller
// configures the in-process dispatch worker without reaching for `delonix_seiva`.
pub use delonix_seiva::worker::{WorkerConfig, WorkerReport};

// The in-memory outbox double needs interior mutability behind the shared `Arc`;
// gated so a `--no-default-features` build (no `testing`) imports nothing unused.
#[cfg(any(test, feature = "testing"))]
use std::sync::Mutex;

/// How long a per-subscriber inbox claim stays valid in [`dispatch`].
///
/// The inbox entry only needs to outlive the window in which the same event can
/// be redelivered; a day is a generous, safe default for the in-process path. It
/// is a constant here because the offline dispatch core takes no configuration;
/// the Fase 2 outbox driver (over the DORM-backed store) is where a deployment
/// tunes the window and purges expired keys (`IdempotencyStore::purge_expired`,
/// ADR 0011).
pub const INBOX_TTL: Duration = Duration::from_secs(60 * 60 * 24);

/// A module's reaction to a single, already-deserialised [`CloudEvent`].
///
/// A subscriber is a plain handler: it receives the envelope another module
/// published and performs its own effect (e.g. `inventory` reserving stock for an
/// `orders.placed` event). It names **no** other module's crate — the
/// composition root is what routes an event type to this handler, which is
/// exactly the independence that lets `seiva module split` be a composition
/// change rather than a rewrite.
///
/// The method is async in the style of the ports (a `Send` future, returning
/// [`PortResult<()>`]). It is object-safe — subscribers are stored behind
/// `Arc<dyn EventSubscriber>` at the composition edge — so the asynchrony is
/// erased with `async-trait`; the handler body itself is ordinary typed code.
///
/// **Identity.** [`subscriber_id`](EventSubscriber::subscriber_id) is the stable
/// name (e.g. `"inventory"`) that, with the event id, composes this subscriber's
/// [`inbox_key`] — so two different subscribers reacting to the same event each
/// get their own consume-once guarantee, and one subscriber never consumes the
/// same event twice.
///
/// **Transaction (Fase 2).** In the integrated driver this handler runs inside
/// its **own** `UnitOfWork::transaction`, together with the inbox claim and mark
/// (effect + mark commit atomically, ack after commit) — the consume pattern of
/// ADR 0011. The offline [`dispatch`] here keeps the same ordering (claim →
/// handle → mark) without a transaction, which is enough to prove the logic.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// The stable identity that composes this subscriber's [`inbox_key`] — e.g.
    /// `"inventory"`. It must be unique per logical subscriber and stable across
    /// restarts, since it is half of the consume-once key.
    fn subscriber_id(&self) -> &str;

    /// React to `event`. Returns `Ok(())` once the effect is done, or an error to
    /// signal the delivery failed (so it may be retried on a later redelivery).
    async fn handle(&self, event: &CloudEvent) -> PortResult<()>;
}

/// The composition root's routing table: which subscribers react to which event
/// type.
///
/// Subscribers are indexed by the CloudEvent `type` string (the envelope's
/// `type`, e.g. `"orders.placed"`), and several subscribers may be registered for
/// the same type. Registration happens once at the composition root, by type —
/// never by importing another module's crate.
///
/// Because subscribers are plugged in at the root and discovered at runtime, some
/// dynamic indirection (`Arc<dyn EventSubscriber>`) is unavoidable here. ADR 0021
/// accepts it and **confines it to the composition/dispatch edge**: it never
/// touches a use case's own statically-typed dispatch path.
#[derive(Clone, Default)]
pub struct SubscriberRegistry {
    by_type: HashMap<String, Vec<Arc<dyn EventSubscriber>>>,
}

impl SubscriberRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `subscriber` to react to `event_type` (the CloudEvent `type`).
    ///
    /// Idempotent in shape but additive in effect: calling it again with the same
    /// type appends another subscriber, so multiple modules can react to one
    /// event. The subscriber's own [`subscriber_id`](EventSubscriber::subscriber_id)
    /// — not the registration order — is what keys its inbox, so registering the
    /// same subscriber twice would deliver to it twice (the composition root owns
    /// that choice).
    pub fn register(
        &mut self,
        event_type: impl Into<String>,
        subscriber: Arc<dyn EventSubscriber>,
    ) {
        self.by_type
            .entry(event_type.into())
            .or_default()
            .push(subscriber);
    }

    /// The subscribers registered for `event_type`, in registration order; empty
    /// if none — so an event no module reacts to is simply a no-op in [`dispatch`].
    #[must_use]
    pub fn subscribers_for(&self, event_type: &str) -> &[Arc<dyn EventSubscriber>] {
        self.by_type
            .get(event_type)
            .map_or(&[], |subscribers| subscribers.as_slice())
    }
}

/// The in-process dispatcher: deliver one `event` to every subscriber registered
/// for its [type](seiva_messaging::CloudEvent::ty), deduplicating each delivery
/// through the `inbox`.
///
/// For each subscriber this claims `inbox_key(subscriber.subscriber_id(),
/// &event.id)` through [`IdempotencyStore::begin`] and acts on the outcome:
///
/// - [`Fresh`](IdempotencyOutcome::Fresh) — first delivery: run the subscriber,
///   then [`finish`](IdempotencyStore::finish) the key so a redelivery is skipped;
/// - [`Replay`](IdempotencyOutcome::Replay) /
///   [`InProgress`](IdempotencyOutcome::InProgress) — the event was already
///   consumed by this subscriber (a redelivery): perform **no** effect;
/// - [`Mismatch`](IdempotencyOutcome::Mismatch) — the inbox key was reused with a
///   different fingerprint, which must never happen for a given event id: return
///   an [`InboxMismatch`] error without running the subscriber.
///
/// The event id is used as the fingerprint, so the same event always resolves to
/// the same claim. The result is at-least-once delivery with exactly-one *effect*
/// per `(subscriber, event)` — identical to the broker drain
/// (`seiva_outbox_relay::drain`). A subscriber that errors propagates that error
/// (subsequent subscribers for this event are not run); the event stays
/// un-acked for the driver to redeliver, and under the Fase 2 transaction a
/// failed handler also rolls back its inbox claim so the retry is `Fresh` again.
///
/// This core does not read the outbox or loop; the Fase 2 outbox driver calls
/// this once per drained event (see the crate docs).
pub async fn dispatch<S, U>(
    event: &CloudEvent,
    registry: &SubscriberRegistry,
    inbox: &S,
    uow: &U,
) -> PortResult<()>
where
    S: IdempotencyStore,
    U: UnitOfWork,
{
    for subscriber in registry.subscribers_for(&event.ty) {
        let key = inbox_key(subscriber.subscriber_id(), &event.id);
        // Did *this* transaction take the key `Fresh`? Only then may we `release` it
        // on failure — never a `Replay`/`InProgress`/`Mismatch` key we do not own, so
        // a concurrent in-flight claim is never disturbed (ADR 0021 Fase 2).
        let claimed_fresh = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // The whole consume — claim → handle → mark — runs in ONE transaction, the
        // consume pattern of ADR 0011. With the DORM gateway the inbox claim, the
        // handler's writes and the `finish` commit atomically, and a rollback
        // un-claims the key for free; the in-memory double does not share this
        // transaction, so on the `Err` path below we `release` the key explicitly.
        //
        // The scope is `Global` **because the CloudEvent envelope carries no tenant
        // in Fase 2** (ADR 0021 addendum): the inbox claim is the driver's own
        // bookkeeping, with no tenant to scope it to. This `Global` is correct ONLY
        // while that premise holds. ADR 0027 (typed, versioned event contract) moves
        // the envelope toward a typed shape; the day `seiva_messaging::CloudEvent`
        // grows a tenant attribute, this claim and the handler transaction should
        // almost certainly run under `Scope::Tenant(event's tenant)`, not `Global` —
        // otherwise a subscriber's writes would silently bypass tenant scoping. That
        // decision must be made deliberately, never inherited by default: the
        // exhaustive-destructure fence in this crate's tests
        // (`scope_global_in_dispatch_is_fenced_to_the_tenantless_envelope`) stops
        // compiling the moment the field is added, turning the review on here.
        let fresh = Arc::clone(&claimed_fresh);
        let outcome = uow
            .transaction(Scope::Global, || async move {
                // The event id is the fingerprint: the same event always claims the
                // same key with the same fingerprint, so a redelivery is never a
                // Mismatch.
                match inbox.begin(&key, &event.id, INBOX_TTL).await? {
                    IdempotencyOutcome::Fresh => {
                        fresh.store(true, std::sync::atomic::Ordering::SeqCst);
                        // Order mirrors the broker consume path (claim → effect →
                        // mark); these now commit together in this transaction.
                        subscriber.handle(event).await?;
                        inbox.finish(&key, &[]).await?;
                        Ok(())
                    }
                    // Already consumed by this subscriber — a redelivery, no effect.
                    IdempotencyOutcome::Replay(_) | IdempotencyOutcome::InProgress => Ok(()),
                    IdempotencyOutcome::Mismatch => Err(Box::new(InboxMismatch {
                        subscriber_id: subscriber.subscriber_id().to_owned(),
                        event_id: event.id.clone(),
                    })
                        as seiva_ports::BoxError),
                }
            })
            .await;

        if let Err(err) = outcome {
            // The transaction rolled back. If it was *our* `Fresh` claim that failed
            // mid-effect, un-claim it so the redelivery is `Fresh` again instead of a
            // stuck `InProgress` that would be silently skipped. On the DORM store
            // this is a no-op (the rollback already dropped the claim row).
            if claimed_fresh.load(std::sync::atomic::Ordering::SeqCst) {
                let release_key = inbox_key(subscriber.subscriber_id(), &event.id);
                // A failing `release` must not mask the handler failure that caused
                // this un-claim: `err` is the real cause the retry/dead-letter path
                // reports, so log the release problem and still return `err`.
                if let Err(release_err) = inbox.release(&release_key).await {
                    tracing::warn!(
                        subscriber_id = subscriber.subscriber_id(),
                        event.id = %event.id,
                        error = %release_err,
                        "failed to release the inbox key after a rolled-back delivery; \
                         the original handler error is preserved",
                    );
                }
            }
            return Err(err);
        }
    }
    Ok(())
}

/// The inbox key for a `(subscriber, event)` was reused with a different
/// fingerprint — an invariant violation, since a given event id must always pin
/// the same claim. [`dispatch`] returns this instead of running the subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxMismatch {
    /// The subscriber whose inbox claim mismatched.
    pub subscriber_id: String,
    /// The event id that produced the mismatching claim.
    pub event_id: String,
}

impl std::fmt::Display for InboxMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "inbox fingerprint mismatch for subscriber `{}` on event `{}`",
            self.subscriber_id, self.event_id
        )
    }
}

impl std::error::Error for InboxMismatch {}

/// One record read from an outbox: a [`CloudEvent`] plus the opaque `position` that
/// [`OutboxReader::advance`] uses to mark it (and everything before it) dispatched.
///
/// The `position` is a monotonically increasing cursor token owned by the reader
/// (the outbox row id in the DORM adapter). The driver never interprets it — it only
/// hands it back to [`advance`](OutboxReader::advance) once the event's
/// [`dispatch`] has returned `Ok`.
#[derive(Clone, Debug)]
pub struct OutboxRecord {
    /// The reader's opaque cursor token for this event. Pass it to
    /// [`OutboxReader::advance`] after the event has been dispatched.
    pub position: u64,
    /// The event to dispatch, in the **same** CloudEvents envelope the broker path
    /// carries — which is what makes a subscriber handler byte-identical across the
    /// in-process and the broker paths.
    pub event: CloudEvent,
}

/// The read side of the outbox the in-process driver drains — the **DORM-free seam**
/// that lets [`drain_in_process`] run against an in-memory double offline and, in the
/// DORM adapter, against the real `dorm-outbox` cursor.
///
/// It mirrors `seiva_outbox_relay::drain` / `dorm_outbox::relay`, but splits the two
/// steps so the *driver* controls their ordering for **at-least-once** delivery:
///
/// 1. [`read_batch`](Self::read_batch) returns the next events the cursor has not
///    passed yet, oldest first;
/// 2. [`advance`](Self::advance) moves the cursor past an event — called **only
///    after** that event's [`dispatch`] returned `Ok`, so a crash (or a failing
///    subscriber) between the two re-reads the event on the next pass rather than
///    silently skipping it.
///
/// An adapter over the in-database outbox implements this in `seiva-outbox-relay`
/// behind the DORM coupling; [`InMemoryOutbox`] implements it for the offline proof.
#[async_trait]
pub trait OutboxReader {
    /// The next up-to-`max` undispatched events, oldest first (cursor-ascending). An
    /// empty batch means the outbox is drained up to the cursor.
    async fn read_batch(&self, max: usize) -> PortResult<Vec<OutboxRecord>>;

    /// Advance the cursor to `position`, acknowledging every event up to and
    /// including it as dispatched. **Monotonic**: a `position` at or behind the
    /// cursor is a no-op, so a slower concurrent pass can never rewind it.
    async fn advance(&self, position: u64) -> PortResult<()>;
}

/// How many events [`drain_in_process`] pulls per pass. A pass dispatches at most
/// this many events, then returns; a worker loops on it, sleeping when it returns
/// `0`. The constant lives here because the offline driver takes no configuration;
/// the DORM adapter reads its batch size from the relay's `RelayConfig` instead.
pub const DRAIN_BATCH: usize = 128;

/// How many times [`drain_in_process`] retries a single event, and how long it
/// waits between attempts, before diverting it to the [`DeadLetterSink`] (ADR 0021
/// Fase 2).
///
/// Retries are **per event, within one drain pass**: a subscriber that errors is
/// re-dispatched up to `max_attempts` times with an exponential back-off
/// (`base_backoff`, then `2×`, `4×`, …), because the Fase 2 transactional dispatch
/// un-claims the inbox key on each failure so every attempt starts `Fresh`. An event
/// that still fails after the last attempt is **dead-lettered** and the cursor
/// advances past it, so it can never block the events behind it. This is what keeps a
/// poison event from either being silently dropped (the Fase 1 bug) or stalling the
/// whole queue forever.
///
/// The back-off sleeps the single drain loop while one event waits — acceptable for
/// the in-process driver's scale (ADR 0021 Fase 2, "Harder / debt"). The broker path
/// keeps its own retry semantics; this governs only the in-process driver.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Total delivery attempts for one event before dead-lettering. `1` means no
    /// retry: a single failure dead-letters at once. Must be `>= 1`; `0` is treated
    /// as `1`.
    pub max_attempts: u32,
    /// The wait before the **second** attempt; each subsequent wait doubles. With
    /// [`Duration::ZERO`] the retries are immediate (what offline tests use).
    pub base_backoff: Duration,
}

impl RetryPolicy {
    /// A policy that retries up to `max_attempts` times with an exponential
    /// `base_backoff`. `max_attempts` is clamped to at least `1`.
    #[must_use]
    pub fn new(max_attempts: u32, base_backoff: Duration) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            base_backoff,
        }
    }

    /// No retry: the first failure dead-letters the event immediately.
    #[must_use]
    pub const fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            base_backoff: Duration::ZERO,
        }
    }

    /// The back-off before attempt number `attempt` (1-based). Attempt `1` waits
    /// nothing; attempt `n` waits `base_backoff * 2^(n-2)`, saturating.
    fn backoff_before(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }
        let shift = attempt - 2;
        self.base_backoff
            .checked_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX))
            .unwrap_or(Duration::MAX)
    }
}

impl Default for RetryPolicy {
    /// Five attempts with a 100 ms base back-off (100 ms, 200 ms, 400 ms, 800 ms) —
    /// a sane production default for a transient-failure window.
    fn default() -> Self {
        Self::new(5, Duration::from_millis(100))
    }
}

/// Where [`drain_in_process`] sends an event that still failed after the
/// [`RetryPolicy`]'s attempts were exhausted — the quarantine that stops a poison
/// event from blocking the queue (ADR 0021 Fase 2).
///
/// It is the in-process twin of a broker's dead-letter queue: the driver hands over
/// the un-dispatchable event (with the attempt count and the last error) and then
/// advances the cursor past it, so the choreography keeps progressing. A sink can log
/// it ([`LoggingDeadLetterSink`]), append it to a table, or publish it to a real DLQ
/// topic; the choice is the composition root's.
#[async_trait]
pub trait DeadLetterSink: Send + Sync {
    /// Record `event`, which failed `attempts` times, with the last error rendered to
    /// `error`. Returning `Err` aborts the drain pass (the cursor does **not** advance
    /// past the event), so a sink that cannot durably accept the event does not let it
    /// be lost — the driver re-reads it on the next pass.
    async fn dead_letter(&self, event: &CloudEvent, attempts: u32, error: &str) -> PortResult<()>;
}

/// The production-default [`DeadLetterSink`]: records the dead-lettered event through
/// the `tracing` facade at `error` level (its id, type, attempt count and the last
/// error) and returns `Ok`, so the drain advances past the poison event.
///
/// It keeps no state and needs no backing store, so a monolith gets a safe default
/// (the event is quarantined and surfaced in the logs) with no extra wiring; a
/// deployment that needs durable dead-lettering swaps in its own sink.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoggingDeadLetterSink;

#[async_trait]
impl DeadLetterSink for LoggingDeadLetterSink {
    async fn dead_letter(&self, event: &CloudEvent, attempts: u32, error: &str) -> PortResult<()> {
        tracing::error!(
            event.id = %event.id,
            event.ty = %event.ty,
            attempts,
            error,
            "in-process dispatch dead-lettered a poison event after exhausting retries",
        );
        Ok(())
    }
}

/// The in-process outbox **driver**: one pass that drains a batch from `reader`,
/// [`dispatch`]es each event to `registry`'s subscribers (deduped through `inbox`),
/// and advances the cursor past each event **only after** its dispatch succeeds.
/// Returns how many events were dispatched this pass (`0` when the outbox is already
/// drained) — a worker loops on it and sleeps on `0`.
///
/// This is the DORM-free twin of `seiva_outbox_relay::drain`: the same at-least-once
/// contract and the same inbox dedup, so a subscriber reacts identically whether a
/// monolith runs this driver or a split service drains the broker — which is what
/// makes `seiva module split` a swap of dispatcher rather than a rewrite (ADR 0021).
///
/// **At-least-once, with bounded retry.** For each event the driver [`dispatch`]es
/// it inside `uow`'s transaction (claim → handle → mark together, ADR 0021 Fase 2)
/// and, on success, advances the cursor past it. On failure it retries up to
/// `policy.max_attempts` with the policy's exponential back-off — each attempt starts
/// `Fresh` because the failed transaction un-claimed the inbox key. An event that
/// still fails after the last attempt is diverted to `dead_letter` and the cursor
/// advances past it, so a poison event can never block the events behind it. A
/// redelivery that *does* get through is absorbed by the inbox: the effect runs
/// exactly once per `(subscriber, event)`.
///
/// The count returned is how many events left the outbox this pass — those dispatched
/// **and** those dead-lettered (both advance the cursor) — so a worker loops while the
/// outbox drains and sleeps when it reaches `0`. The only early return is a failure of
/// the infrastructure itself (`reader`/`dead_letter` erroring); a subscriber failure
/// is absorbed by retry-then-dead-letter, never propagated.
pub async fn drain_in_process<R, S, U, D>(
    reader: &R,
    registry: &SubscriberRegistry,
    inbox: &S,
    uow: &U,
    policy: &RetryPolicy,
    dead_letter: &D,
) -> PortResult<usize>
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
    U: UnitOfWork,
    D: DeadLetterSink,
{
    let batch = reader.read_batch(DRAIN_BATCH).await?;
    let mut drained = 0usize;
    for record in batch {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let backoff = policy.backoff_before(attempt);
            if !backoff.is_zero() {
                tokio::time::sleep(backoff).await;
            }
            match dispatch(&record.event, registry, inbox, uow).await {
                Ok(()) => {
                    // Dispatched cleanly — advance the cursor past it (at-least-once:
                    // the advance follows a successful dispatch, never precedes it).
                    reader.advance(record.position).await?;
                    drained += 1;
                    break;
                }
                Err(err) if attempt >= policy.max_attempts => {
                    // Exhausted the retries: quarantine the poison event and advance
                    // past it so the events behind it still progress. A dead-letter
                    // sink that errors aborts the pass WITHOUT advancing, so the event
                    // is not lost — it is re-read next pass.
                    dead_letter
                        .dead_letter(&record.event, attempt, &err.to_string())
                        .await?;
                    reader.advance(record.position).await?;
                    drained += 1;
                    break;
                }
                // A failed attempt with retries left: the transaction already
                // un-claimed the inbox key, so looping re-dispatches it `Fresh`.
                Err(_) => continue,
            }
        }
    }
    Ok(drained)
}

/// Run [`drain_in_process`] in a loop until `shutdown` resolves — the in-process
/// twin of `seiva_outbox_relay::run_relay_worker`, sharing the framework's worker
/// runtime ([`run_worker`]): an interval between passes, a `drain_threshold` to run
/// straight on while clearing a backlog, graceful drain-and-exit on shutdown, and a
/// [`WorkerReport`] of passes/processed/errors. A failing pass is logged and counted
/// but **never fatal** — the worker outlives a transient outbox or store error.
///
/// This is how a monolith makes the choreography progress on its own: a use case
/// that publishes `orders.placed` into the outbox need not call [`drain_in_process`]
/// by hand — this worker drains the outbox continuously, so a registered subscriber
/// reacts with no further wiring. Swapping it for `run_relay_worker` (same shape,
/// same subscriber) is exactly the `module split` from monolith to microservice:
/// only the dispatcher changes, not the subscriber and not the guarantees.
// This is a top-level composition entry point: it takes the independent collaborators
// a composition root supplies by hand — the outbox reader, the subscriber registry,
// the inbox, the unit of work, the retry policy and the dead-letter sink, plus the
// worker config and shutdown signal. Grouping them into a struct would only move the
// same wiring to the caller, so the argument list stands.
#[allow(clippy::too_many_arguments)]
pub async fn run_in_process_dispatch_worker<R, S, U, D, Sig>(
    config: WorkerConfig,
    shutdown: Sig,
    reader: R,
    registry: SubscriberRegistry,
    inbox: S,
    uow: U,
    policy: RetryPolicy,
    dead_letter: D,
) -> WorkerReport
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
    U: UnitOfWork,
    D: DeadLetterSink,
    Sig: Future<Output = ()>,
{
    let reader = Arc::new(reader);
    let inbox = Arc::new(inbox);
    let registry = Arc::new(registry);
    let uow = Arc::new(uow);
    let dead_letter = Arc::new(dead_letter);
    run_worker(config, shutdown, move || {
        // Each pass owns its clones, so the future borrows only its own locals and
        // stays `Send` across `.await` — the same shape as the relay worker.
        let reader = Arc::clone(&reader);
        let inbox = Arc::clone(&inbox);
        let registry = Arc::clone(&registry);
        let uow = Arc::clone(&uow);
        let dead_letter = Arc::clone(&dead_letter);
        async move {
            let drained =
                drain_in_process(&*reader, &registry, &*inbox, &*uow, &policy, &*dead_letter)
                    .await?;
            Ok(drained as u64)
        }
    })
    .await
}

/// An in-memory [`OutboxReader`] double (behind the `testing` feature): an
/// append-only log of [`CloudEvent`]s with a single advancing cursor, so the
/// choreography can be proved with no database and no broker.
///
/// A test plays the role of a publishing module by calling [`publish`](Self::publish)
/// — the in-memory stand-in for a use case writing to the transactional outbox — then
/// drives [`drain_in_process`] over it. [`read_batch`](OutboxReader::read_batch)
/// returns everything after the cursor; [`advance`](OutboxReader::advance) moves the
/// cursor forward; [`redeliver`](Self::redeliver) rewinds it to model an
/// at-least-once **redelivery** (a crash after dispatch but before the cursor
/// advanced), so a test can prove the inbox absorbs it independently of the cursor.
///
/// It is a single-process model (a `Mutex`, nothing durable); the real durable,
/// cursor-per-consumer guarantee is `dorm-outbox`, exercised by the DORM adapter.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct InMemoryOutbox {
    inner: Arc<Mutex<InMemoryOutboxState>>,
}

#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
struct InMemoryOutboxState {
    /// Appended events, each with its 1-based position (its cursor id).
    records: Vec<OutboxRecord>,
    /// The highest position dispatched; `read_batch` returns records beyond it.
    cursor: u64,
}

#[cfg(any(test, feature = "testing"))]
impl InMemoryOutbox {
    /// An empty outbox with the cursor at the start.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `event` as a publishing module would into the transactional outbox,
    /// returning its position. Positions increase monotonically, so the cursor
    /// orders deliveries exactly as `dorm-outbox`'s row ids do.
    pub fn publish(&self, event: CloudEvent) -> u64 {
        let mut st = self.inner.lock().expect("InMemoryOutbox poisoned");
        let position = st.records.len() as u64 + 1;
        st.records.push(OutboxRecord { position, event });
        position
    }

    /// How many events are still ahead of the cursor (undispatched).
    #[must_use]
    pub fn pending(&self) -> usize {
        let st = self.inner.lock().expect("InMemoryOutbox poisoned");
        st.records.iter().filter(|r| r.position > st.cursor).count()
    }

    /// Rewind the cursor to `position` to model an at-least-once **redelivery**: the
    /// events after `position` are read again, exactly as if the process had crashed
    /// after dispatching them but before the cursor advanced. The inbox is what then
    /// keeps the effect exactly-once. (Never moves the cursor forward.)
    pub fn redeliver(&self, position: u64) {
        let mut st = self.inner.lock().expect("InMemoryOutbox poisoned");
        st.cursor = position.min(st.cursor);
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl OutboxReader for InMemoryOutbox {
    async fn read_batch(&self, max: usize) -> PortResult<Vec<OutboxRecord>> {
        let st = self.inner.lock().expect("InMemoryOutbox poisoned");
        Ok(st
            .records
            .iter()
            .filter(|r| r.position > st.cursor)
            .take(max)
            .cloned()
            .collect())
    }

    async fn advance(&self, position: u64) -> PortResult<()> {
        let mut st = self.inner.lock().expect("InMemoryOutbox poisoned");
        if position > st.cursor {
            st.cursor = position;
        }
        Ok(())
    }
}

/// One event the in-process driver gave up on, captured by [`InMemoryDeadLetterQueue`]:
/// the event, how many attempts failed, and the last error rendered to a string.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Debug)]
pub struct DeadLettered {
    /// The event that could not be dispatched after the retries were exhausted.
    pub event: CloudEvent,
    /// How many delivery attempts failed before it was dead-lettered.
    pub attempts: u32,
    /// The last error, rendered with `Display`.
    pub error: String,
}

/// An in-memory [`DeadLetterSink`] double (behind the `testing` feature): records
/// every dead-lettered event so a test can assert a poison event was quarantined and
/// the choreography kept going.
///
/// It is the observable twin of a real dead-letter queue — [`drain_in_process`] hands
/// it the un-dispatchable event and advances past it, so a test proves "the poison
/// event goes to dead-letter, the next event processes" with no broker and no database.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct InMemoryDeadLetterQueue {
    entries: Arc<Mutex<Vec<DeadLettered>>>,
}

#[cfg(any(test, feature = "testing"))]
impl InMemoryDeadLetterQueue {
    /// An empty dead-letter queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every dead-lettered event recorded so far, oldest first.
    #[must_use]
    pub fn entries(&self) -> Vec<DeadLettered> {
        self.entries
            .lock()
            .expect("InMemoryDeadLetterQueue poisoned")
            .clone()
    }

    /// How many events have been dead-lettered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("InMemoryDeadLetterQueue poisoned")
            .len()
    }

    /// Whether no event has been dead-lettered yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl DeadLetterSink for InMemoryDeadLetterQueue {
    async fn dead_letter(&self, event: &CloudEvent, attempts: u32, error: &str) -> PortResult<()> {
        self.entries
            .lock()
            .expect("InMemoryDeadLetterQueue poisoned")
            .push(DeadLettered {
                event: event.clone(),
                attempts,
                error: error.to_owned(),
            });
        Ok(())
    }
}

/// An [`EventPublisher`](seiva_ports::EventPublisher) that writes into an
/// [`InMemoryOutbox`] (behind the `testing` feature): the publish side of the
/// offline choreography, the in-memory stand-in for a use case writing the
/// transactional outbox.
///
/// It is what a module's application service publishes through in the offline
/// proof. The port's contract is deliberately byte-oriented — the application
/// hands it an *already-serialised* payload and a topic, owning the envelope
/// format — so this adapter is the piece that lifts those bytes back into the
/// CloudEvents envelope the dispatcher and the broker path both carry:
///
/// 1. it mints a unique event id, `format!("{source}-{n}")`, from a fixed
///    [`source`](Self::new) and a monotonic counter — so every publish is a
///    distinct event the inbox can dedup on;
/// 2. it parses the payload as JSON into the CloudEvent `data`; a payload that is
///    not valid JSON is not an error here — it is wrapped as a JSON string so no
///    event is ever dropped (the application still owns whether it emits JSON);
/// 3. it builds [`CloudEvent::new(id, source, topic, data)`](CloudEvent) — the
///    `topic` becomes the envelope `type` the [`SubscriberRegistry`] routes on —
///    and appends it to the shared outbox.
///
/// The outbox is held behind an [`Arc`] so the same log the publisher writes is
/// the one [`drain_in_process`] reads: the composition root clones one
/// `Arc<InMemoryOutbox>` into this publisher (injected into the producing module)
/// and into the driver. Cloning the publisher is **not** offered on purpose —
/// each logical producer owns its own id sequence, so one publisher per injected
/// module keeps event ids collision-free under its `source`.
#[cfg(any(test, feature = "testing"))]
pub struct InMemoryOutboxPublisher {
    /// The shared log every published event is appended to; the twin the driver
    /// reads from.
    outbox: Arc<InMemoryOutbox>,
    /// The fixed CloudEvents `source` for every event (and the id prefix), e.g.
    /// `"/shop/orders"`.
    source: String,
    /// Monotonic per-publisher counter that, with `source`, composes each unique
    /// event id.
    counter: std::sync::atomic::AtomicU64,
}

#[cfg(any(test, feature = "testing"))]
impl InMemoryOutboxPublisher {
    /// Builds a publisher that appends to `outbox`, stamping every event with
    /// `source` as both its CloudEvents `source` and the prefix of its generated
    /// id. Clone the `outbox` `Arc` first so the driver shares the same log.
    #[must_use]
    pub fn new(outbox: Arc<InMemoryOutbox>, source: impl Into<String>) -> Self {
        Self {
            outbox,
            source: source.into(),
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl seiva_ports::EventPublisher for InMemoryOutboxPublisher {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()> {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let id = format!("{}-{n}", self.source);
        // The payload is already serialised by the producing layer; lift it back
        // into the envelope `data`. A non-JSON payload is wrapped rather than
        // rejected, so a publish never fails on the shape of the bytes.
        let data = serde_json::from_slice(payload).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(payload).into_owned())
        });
        self.outbox
            .publish(CloudEvent::new(id, &self.source, topic, data));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_ports::{BoxError, EventPublisher};
    use seiva_testing::{InMemoryIdempotencyStore, InMemoryUnitOfWork};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A subscriber that counts how many times it was actually run — the "effect".
    #[derive(Clone)]
    struct CountingSubscriber {
        id: String,
        runs: Arc<AtomicUsize>,
    }

    impl CountingSubscriber {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_owned(),
                runs: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventSubscriber for CountingSubscriber {
        fn subscriber_id(&self) -> &str {
            &self.id
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A subscriber whose effect always fails — to prove errors propagate.
    struct FailingSubscriber;

    #[async_trait]
    impl EventSubscriber for FailingSubscriber {
        fn subscriber_id(&self) -> &str {
            "faulty"
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            Err(BoxError::from("subscriber boom"))
        }
    }

    /// A subscriber that fails its first `fail_first` deliveries, then succeeds —
    /// the flaky handler that proves the inbox key is un-claimed on a rolled-back
    /// delivery (so the retry is `Fresh`, not a stuck `InProgress`). `runs` counts
    /// successful handles.
    #[derive(Clone)]
    struct FlakySubscriber {
        id: String,
        attempts: Arc<AtomicUsize>,
        runs: Arc<AtomicUsize>,
        fail_first: usize,
    }

    impl FlakySubscriber {
        fn new(id: &str, fail_first: usize) -> Self {
            Self {
                id: id.to_owned(),
                attempts: Arc::new(AtomicUsize::new(0)),
                runs: Arc::new(AtomicUsize::new(0)),
                fail_first,
            }
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventSubscriber for FlakySubscriber {
        fn subscriber_id(&self) -> &str {
            &self.id
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            let n = self.attempts.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_first {
                return Err(BoxError::from("flaky subscriber transient failure"));
            }
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A subscriber that fails only for the event whose id is `poison`, so a single
    /// poison event can be mixed with healthy ones in one outbox.
    #[derive(Clone)]
    struct PoisonForId {
        id: String,
        poison: String,
        runs: Arc<AtomicUsize>,
    }

    impl PoisonForId {
        fn new(id: &str, poison: &str) -> Self {
            Self {
                id: id.to_owned(),
                poison: poison.to_owned(),
                runs: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn runs(&self) -> usize {
            self.runs.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventSubscriber for PoisonForId {
        fn subscriber_id(&self) -> &str {
            &self.id
        }

        async fn handle(&self, event: &CloudEvent) -> PortResult<()> {
            if event.id == self.poison {
                return Err(BoxError::from("poison event always fails"));
            }
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn event(ty: &str, id: &str) -> CloudEvent {
        CloudEvent::new(id, "/shop/orders", ty, serde_json::Value::Null)
    }

    // ADR 0027 — compile-fence for the `Scope::Global` pinned in `dispatch`.
    //
    // `dispatch` runs the inbox claim + handler transaction under `Scope::Global`
    // *because the CloudEvent envelope carries no tenant in Fase 2* (ADR 0021
    // addendum). ADR 0027 moves the envelope toward a typed, versioned contract, and
    // the day `seiva_messaging::CloudEvent` grows a tenant attribute that `Global`
    // stops being obviously correct: the consume should almost certainly run under
    // `Scope::Tenant(…)`, or a subscriber's writes would silently bypass tenant
    // scoping.
    //
    // The destructuring below is **exhaustive on purpose and carries no `..`**: the
    // moment a field is added to `CloudEvent`, this stops compiling (E0027), which
    // turns `cargo test -p seiva-events` red and sends whoever added the field to the
    // `Scope::Global` site in `dispatch` to pick the scope deliberately instead of
    // inheriting `Global`. Each binding is `_`-prefixed so the fence asserts the
    // *shape* of the envelope without constraining what the fields mean.
    //
    // Proof it bites: add `pub tenant: Option<String>` to `CloudEvent` with no other
    // change and this test fails with "pattern does not mention field `tenant`".
    #[test]
    fn scope_global_in_dispatch_is_fenced_to_the_tenantless_envelope() {
        let CloudEvent {
            id: _id,
            source: _source,
            ty: _ty,
            time: _time,
            traceparent: _traceparent,
            data: _data,
        } = event("orders.placed", "evt-1");
    }

    #[tokio::test]
    async fn one_event_runs_every_subscriber_of_its_type() {
        let inventory = CountingSubscriber::new("inventory");
        let shipping = CountingSubscriber::new("shipping");
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(inventory.clone()));
        registry.register("orders.placed", Arc::new(shipping.clone()));

        let inbox = InMemoryIdempotencyStore::new();
        let uow = InMemoryUnitOfWork::new();
        dispatch(&event("orders.placed", "evt-1"), &registry, &inbox, &uow)
            .await
            .unwrap();

        // Two subscribers of the SAME type both ran once.
        assert_eq!(inventory.runs(), 1);
        assert_eq!(shipping.runs(), 1);
    }

    #[tokio::test]
    async fn subscribers_of_other_types_are_not_run() {
        let inventory = CountingSubscriber::new("inventory");
        let notifier = CountingSubscriber::new("notifications");
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(inventory.clone()));
        registry.register("payments.captured", Arc::new(notifier.clone()));

        dispatch(
            &event("orders.placed", "evt-1"),
            &registry,
            &InMemoryIdempotencyStore::new(),
            &InMemoryUnitOfWork::new(),
        )
        .await
        .unwrap();

        assert_eq!(inventory.runs(), 1);
        assert_eq!(
            notifier.runs(),
            0,
            "a different type's subscriber is untouched"
        );
    }

    #[tokio::test]
    async fn redelivery_of_the_same_event_runs_each_subscriber_once() {
        let inventory = CountingSubscriber::new("inventory");
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(inventory.clone()));

        // One shared inbox across both deliveries — the dedup authority.
        let inbox = InMemoryIdempotencyStore::new();
        let uow = InMemoryUnitOfWork::new();
        let ev = event("orders.placed", "evt-1");
        dispatch(&ev, &registry, &inbox, &uow).await.unwrap();
        dispatch(&ev, &registry, &inbox, &uow).await.unwrap();

        assert_eq!(
            inventory.runs(),
            1,
            "at-least-once delivery, exactly-one effect per (subscriber, event)"
        );
    }

    #[tokio::test]
    async fn an_event_with_no_subscribers_is_a_noop() {
        let registry = SubscriberRegistry::new();
        // No panic, no error, nothing to run.
        dispatch(
            &event("orders.placed", "evt-1"),
            &registry,
            &InMemoryIdempotencyStore::new(),
            &InMemoryUnitOfWork::new(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_failing_subscriber_propagates_its_error() {
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(FailingSubscriber));

        let err = dispatch(
            &event("orders.placed", "evt-1"),
            &registry,
            &InMemoryIdempotencyStore::new(),
            &InMemoryUnitOfWork::new(),
        )
        .await
        .expect_err("a failing subscriber must surface its error");
        assert!(err.to_string().contains("boom"));
    }

    // ADR 0021 Fase 2 — the transactional dispatch un-claims the inbox key on a
    // rolled-back delivery, so a transient failure is retried `Fresh` instead of
    // being stuck `InProgress` and silently dropped.
    #[tokio::test]
    async fn a_failed_delivery_unclaims_the_key_so_the_redelivery_is_fresh() {
        // A subscriber that fails its first delivery and succeeds on the second.
        let flaky = FlakySubscriber::new("inventory", 1);
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(flaky.clone()));

        // One shared inbox + uow across both deliveries (the real at-least-once path).
        let inbox = InMemoryIdempotencyStore::new();
        let uow = InMemoryUnitOfWork::new();
        let ev = event("orders.placed", "evt-1");

        // First delivery fails; the transaction rolls back and un-claims the key.
        let err = dispatch(&ev, &registry, &inbox, &uow)
            .await
            .expect_err("the first delivery fails");
        assert!(err.to_string().contains("transient"));
        assert_eq!(
            flaky.runs(),
            0,
            "no effect committed on the failed delivery"
        );

        // Redelivery: because the key was un-claimed it is `Fresh` again, so the
        // handler runs — in Fase 1 it would have seen a stuck `InProgress` and been
        // skipped, silently losing the event.
        dispatch(&ev, &registry, &inbox, &uow)
            .await
            .expect("the redelivery succeeds now the handler is healthy");
        assert_eq!(
            flaky.runs(),
            1,
            "the redelivery re-ran the handler exactly once"
        );

        // A further redelivery is now a no-op (the key is finished → Replay).
        dispatch(&ev, &registry, &inbox, &uow).await.unwrap();
        assert_eq!(flaky.runs(), 1, "a third delivery is deduped by the inbox");
    }

    #[tokio::test]
    async fn outbox_publisher_writes_one_event_with_the_topic_as_type() {
        let outbox = Arc::new(InMemoryOutbox::new());
        let publisher = InMemoryOutboxPublisher::new(Arc::clone(&outbox), "/shop/orders");

        let payload = serde_json::to_vec(&serde_json::json!({ "id": "7" })).unwrap();
        // Publish through the PORT, exactly as an application service would.
        publisher.publish("orders.placed", &payload).await.unwrap();

        assert_eq!(
            outbox.pending(),
            1,
            "the publish landed in the shared outbox"
        );
        let batch = outbox.read_batch(10).await.unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(
            batch[0].event.ty, "orders.placed",
            "the topic became the CloudEvent type the registry routes on"
        );
        assert_eq!(batch[0].event.source, "/shop/orders");
        assert_eq!(batch[0].event.data["id"], "7");
    }

    #[tokio::test]
    async fn the_worker_drains_the_outbox_and_stops_on_shutdown() {
        // Three events sit in the outbox; the worker must drain and dispatch them
        // all, then exit when shutdown fires — proving a monolith progresses the
        // choreography on its own, with no manual `drain_in_process` call.
        let outbox = InMemoryOutbox::new();
        for i in 1..=3 {
            outbox.publish(event("orders.placed", &format!("evt-{i}")));
        }

        let inventory = CountingSubscriber::new("inventory");
        let probe = inventory.clone(); // shares the same run counter (an `Arc`)
        let result = inventory.clone();
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(inventory));

        // Stop once every event has been reserved. The worker drains all three in
        // its first pass, so by the time this future is polled (after that pass)
        // the count is already three and the loop exits on the next shutdown check.
        let shutdown = async move {
            while probe.runs() < 3 {
                tokio::task::yield_now().await;
            }
        };

        let report = run_in_process_dispatch_worker(
            WorkerConfig::new("test-in-process-dispatch", Duration::ZERO),
            shutdown,
            outbox,
            registry,
            InMemoryIdempotencyStore::new(),
            InMemoryUnitOfWork::new(),
            RetryPolicy::no_retry(),
            LoggingDeadLetterSink,
        )
        .await;

        assert_eq!(
            result.runs(),
            3,
            "the worker drained and dispatched all three events"
        );
        assert_eq!(
            report.processed, 3,
            "the report accounts for three processed items"
        );
        assert!(report.passes >= 1, "at least one pass ran");
    }

    // ADR 0021 Fase 2 — a poison event that fails every attempt is dead-lettered and
    // the cursor advances past it, so the healthy event behind it still processes.
    #[tokio::test]
    async fn a_poison_event_is_dead_lettered_and_the_next_event_processes() {
        // One subscriber that fails for the event id "poison" and succeeds otherwise.
        let sub = PoisonForId::new("inventory", "poison");
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(sub.clone()));

        // The outbox holds the poison event first, then a healthy one.
        let outbox = InMemoryOutbox::new();
        outbox.publish(event("orders.placed", "poison"));
        outbox.publish(event("orders.placed", "good"));

        let inbox = InMemoryIdempotencyStore::new();
        let uow = InMemoryUnitOfWork::new();
        let dlq = InMemoryDeadLetterQueue::new();
        // Three attempts, no back-off (immediate) so the test is fast.
        let policy = RetryPolicy::new(3, Duration::ZERO);

        let drained = drain_in_process(&outbox, &registry, &inbox, &uow, &policy, &dlq)
            .await
            .unwrap();

        // Both events left the outbox (one dispatched, one dead-lettered).
        assert_eq!(drained, 2, "both events left the outbox this pass");
        assert_eq!(outbox.pending(), 0, "the cursor advanced past both");
        // The healthy event was NOT blocked by the poison one.
        assert_eq!(sub.runs(), 1, "the event behind the poison one processed");
        // The poison event is quarantined, with the exhausted attempt count recorded.
        assert_eq!(dlq.len(), 1, "exactly the poison event was dead-lettered");
        let entry = &dlq.entries()[0];
        assert_eq!(entry.event.id, "poison");
        assert_eq!(entry.attempts, 3, "it was tried the full policy budget");
        assert!(entry.error.contains("poison"));
    }

    // A dead-letter sink that itself errors must NOT advance the cursor — the event
    // is not lost, it is re-read on the next pass.
    #[tokio::test]
    async fn a_failing_dead_letter_sink_leaves_the_event_pending() {
        struct RejectingSink;
        #[async_trait]
        impl DeadLetterSink for RejectingSink {
            async fn dead_letter(
                &self,
                _event: &CloudEvent,
                _attempts: u32,
                _error: &str,
            ) -> PortResult<()> {
                Err(BoxError::from("dead-letter store is down"))
            }
        }

        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(FailingSubscriber));
        let outbox = InMemoryOutbox::new();
        outbox.publish(event("orders.placed", "evt-1"));

        let err = drain_in_process(
            &outbox,
            &registry,
            &InMemoryIdempotencyStore::new(),
            &InMemoryUnitOfWork::new(),
            &RetryPolicy::no_retry(),
            &RejectingSink,
        )
        .await
        .expect_err("a dead-letter sink failure aborts the pass");
        assert!(err.to_string().contains("dead-letter store is down"));
        assert_eq!(
            outbox.pending(),
            1,
            "the event stays pending — not lost — for the next pass"
        );
    }

    #[test]
    fn retry_policy_backoff_is_exponential_and_clamped() {
        let p = RetryPolicy::new(5, Duration::from_millis(100));
        assert_eq!(
            p.backoff_before(1),
            Duration::ZERO,
            "first attempt waits none"
        );
        assert_eq!(p.backoff_before(2), Duration::from_millis(100));
        assert_eq!(p.backoff_before(3), Duration::from_millis(200));
        assert_eq!(p.backoff_before(4), Duration::from_millis(400));
        // max_attempts is clamped to at least 1.
        assert_eq!(RetryPolicy::new(0, Duration::ZERO).max_attempts, 1);
    }
}
