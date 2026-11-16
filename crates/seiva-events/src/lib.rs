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
use seiva_observability::trace_context::{self, TraceContext};
use seiva_ports::{inbox_key, IdempotencyOutcome, IdempotencyStore, PortResult};
use tracing::Instrument;

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
pub async fn dispatch<S: IdempotencyStore>(
    event: &CloudEvent,
    registry: &SubscriberRegistry,
    inbox: &S,
) -> PortResult<()> {
    // Open a handling span and parent it to the trace context the publisher injected
    // onto the envelope, so the emitting use case's span is the parent of this one
    // (ADR 0027) — the trace survives the outbox→broker→subscriber hop. Both are
    // no-ops until a service enables `seiva-observability/tracing` at its root, so
    // the default build pays nothing (`set_parent` does not even touch the span).
    let span =
        tracing::info_span!("seiva.event.dispatch", event_type = %event.ty, event_id = %event.id);
    trace_context::set_parent(
        &span,
        &TraceContext {
            traceparent: event.traceparent.clone(),
            tracestate: event.tracestate.clone(),
        },
    );
    dispatch_to_subscribers(event, registry, inbox)
        .instrument(span)
        .await
}

/// The subscriber fan-out of [`dispatch`], factored out so the handling span wraps
/// exactly the per-subscriber claim → handle → mark loop.
async fn dispatch_to_subscribers<S: IdempotencyStore>(
    event: &CloudEvent,
    registry: &SubscriberRegistry,
    inbox: &S,
) -> PortResult<()> {
    for subscriber in registry.subscribers_for(&event.ty) {
        let key = inbox_key(subscriber.subscriber_id(), &event.id);
        // The event id is the fingerprint: the same event always claims the same
        // key with the same fingerprint, so a redelivery can never be a Mismatch.
        match inbox.begin(&key, &event.id, INBOX_TTL).await? {
            IdempotencyOutcome::Fresh => {
                // Order mirrors the broker consume path (claim → effect → mark).
                // The store here is not transactional, so a failed effect is
                // compensated by releasing the claim (`abort`): the redelivery is
                // `Fresh` again and re-runs the effect, instead of seeing an
                // orphaned `InProgress` and skipping it until the TTL — the
                // silent-loss path this closes (ADR 0028, at-least-once). Where a
                // transactional store wraps claim+effect+mark, its rollback is the
                // compensation and `abort` is a documented no-op.
                if let Err(err) = subscriber.handle(event).await {
                    inbox.abort(&key).await?;
                    return Err(err);
                }
                inbox.finish(&key, &[]).await?;
            }
            // Already consumed by this subscriber — a redelivery, no effect. A
            // durable `InProgress` is only reached here when a concurrent claim is
            // genuinely in flight (a transactional store); the non-transactional
            // in-process path compensates on failure above, so it never strands one.
            IdempotencyOutcome::Replay(_) | IdempotencyOutcome::InProgress => {}
            IdempotencyOutcome::Mismatch => {
                return Err(Box::new(InboxMismatch {
                    subscriber_id: subscriber.subscriber_id().to_owned(),
                    event_id: event.id.clone(),
                }));
            }
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

/// How many events a drain pass pulls by default — the [`RelayPolicy::default`]
/// batch. A pass dispatches at most this many events, then returns; a worker loops
/// on it, sleeping when it returns `0`.
pub const DRAIN_BATCH: usize = 128;

/// How a drain pass behaves on failure — the **in-process twin of
/// `dorm_outbox::RelayConfig`**, so a monolith's in-process driver and a split
/// service's broker relay apply **one** failure policy (max attempts, exponential
/// backoff, then dead-letter), not two drifting sets of constants (ADR 0028, D-2).
///
/// The broker policy lives in the DORM-git-coupled `dorm-outbox`, which this
/// DORM-free crate cannot name; this is its structural equal, carrying the same
/// three knobs with the same meaning and the same [`backoff`](Self::backoff)
/// formula. The DORM boundary (`seiva-outbox-relay`) provides the
/// `From<RelayPolicy> for dorm_outbox::RelayConfig` that makes one `RelayPolicy`
/// drive both paths, so the shared policy is the single source of truth.
///
/// The backoff **formula** is not duplicated here: [`backoff`](Self::backoff)
/// delegates to [`seiva_resilience::RetryPolicy`] (see [`retry_policy`](Self::retry_policy)),
/// so the exponential schedule lives in exactly one crate and the in-process
/// drain, the broker relay and a `seiva-client` inter-service call all pace a
/// transient failure identically — one source, not three drifting copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPolicy {
    /// Events pulled per [`drain_in_process_with`] pass — the `read_batch` size.
    pub batch: usize,
    /// How many times a single event's [`dispatch`] is attempted before it is
    /// dead-lettered and the cursor advances past it. Must be `>= 1`.
    pub max_attempts: u32,
    /// Base backoff between a failing event's attempts; doubles each attempt,
    /// capped at 30× base — the exact schedule `dorm_outbox::relay` sleeps.
    pub backoff_base_ms: u64,
}

impl Default for RelayPolicy {
    /// The same failure policy as `dorm_outbox::RelayConfig::default()` — five
    /// attempts with a 50 ms base backoff — over the in-process [`DRAIN_BATCH`].
    fn default() -> Self {
        Self {
            batch: DRAIN_BATCH,
            max_attempts: 5,
            backoff_base_ms: 50,
        }
    }
}

impl RelayPolicy {
    /// This failure policy's [`seiva_resilience::RetryPolicy`] — the same
    /// `max_attempts` and `backoff_base_ms`, so the shared resilience crate owns
    /// the retry schedule and this policy carries only the extra `batch` knob the
    /// drain pass needs. One place defines the backoff; everything derives from it.
    #[must_use]
    pub fn retry_policy(&self) -> seiva_resilience::RetryPolicy {
        seiva_resilience::RetryPolicy {
            max_attempts: self.max_attempts,
            backoff_base_ms: self.backoff_base_ms,
        }
    }

    /// The backoff before the `attempt`-th retry (1-based), doubling each attempt
    /// and capped at 30× the base — byte-for-byte the schedule `dorm_outbox::relay`
    /// uses, so a transient failure is paced identically in-process and over the
    /// broker.
    ///
    /// Delegates to [`seiva_resilience::RetryPolicy::backoff`]: the formula lives
    /// in the resilience crate, not duplicated here, so the drain, the relay and a
    /// `seiva-client` call share one schedule by construction.
    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        self.retry_policy().backoff(attempt)
    }
}

/// Where an event goes when its [`dispatch`] has failed
/// [`max_attempts`](RelayPolicy::max_attempts) times — the in-process twin of
/// `dorm_outbox`'s dead-letter table (`dorm_outbox_dead`).
///
/// Dead-lettering is what keeps a permanently-failing event from **blocking the
/// head of the line forever**: once the retry budget is spent, the driver hands
/// the event here and advances the cursor past it, exactly as `dorm_outbox::relay`
/// does (ADR 0028). The sink is the seam between the DORM-free driver and where the
/// poisoned event is durably recorded: [`TracingDeadLetters`] only logs it (the
/// offline/demo default), while the DORM adapter in `seiva-outbox-relay` inserts it
/// into the real `dorm_outbox_dead` table so the in-process and broker paths bury a
/// poison pill the same way.
#[async_trait]
pub trait DeadLetterSink {
    /// Record `record` as undeliverable, with the last `error`. The driver advances
    /// the cursor past the event **only after** this returns `Ok`; an `Err` here
    /// leaves the cursor put, so a failing sink re-reads (and re-attempts) the event
    /// rather than dropping it silently.
    async fn dead_letter(&self, record: &OutboxRecord, error: &str) -> PortResult<()>;
}

/// The default [`DeadLetterSink`]: log the poisoned event at `error` level and drop
/// it. This is the **offline/in-memory** default — honest for a demo or a monolith
/// over the non-durable [`InMemoryOutbox`], where there is no table to write. A
/// production monolith draining the real outbox should pass the DORM dead-letter
/// adapter from `seiva-outbox-relay` instead, so a poison pill is **durably** kept
/// rather than only logged.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingDeadLetters;

#[async_trait]
impl DeadLetterSink for TracingDeadLetters {
    async fn dead_letter(&self, record: &OutboxRecord, error: &str) -> PortResult<()> {
        tracing::error!(
            target: "seiva::events",
            event_id = %record.event.id,
            event_type = %record.event.ty,
            error,
            "in-process event dead-lettered after exhausting its retry budget"
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
/// **At-least-once, then dead-letter.** The cursor advances (`reader.advance`) only
/// after an event is fully handled. A failing [`dispatch`] is retried up to
/// [`max_attempts`](RelayPolicy::max_attempts) with the policy's exponential
/// [`backoff`](RelayPolicy::backoff) (the inbox [`abort`](IdempotencyStore::abort)
/// on each failure makes every retry `Fresh`, so a transient failure's effect still
/// runs exactly once). If the retry budget is spent the event is handed to the
/// dead-letter sink and the cursor advances past it, so a poison pill never blocks
/// the head of the line — the **exact** policy `dorm_outbox::relay` applies on the
/// broker path. A redelivery that *does* get through is absorbed by the inbox: the
/// effect runs exactly once per `(subscriber, event)`.
///
/// This convenience uses [`RelayPolicy::default`] and [`TracingDeadLetters`] (the
/// offline/demo defaults); call [`drain_in_process_with`] to supply a tuned policy
/// or a durable dead-letter sink.
pub async fn drain_in_process<R, S>(
    reader: &R,
    registry: &SubscriberRegistry,
    inbox: &S,
) -> PortResult<usize>
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
{
    drain_in_process_with(
        reader,
        registry,
        inbox,
        RelayPolicy::default(),
        &TracingDeadLetters,
    )
    .await
}

/// [`drain_in_process`] with an explicit failure [`policy`](RelayPolicy) and
/// [`dead_letters`](DeadLetterSink) sink — the full driver that lets a deployment
/// tune the retry budget/backoff and route poison pills to a durable store.
///
/// For each drained event it attempts [`dispatch`] up to
/// [`max_attempts`](RelayPolicy::max_attempts), sleeping the policy's
/// [`backoff`](RelayPolicy::backoff) between attempts; on success it advances the
/// cursor, and on a spent budget it dead-letters the event (via `dead_letters`) and
/// advances past it. Returns how many events the cursor advanced past this pass
/// (dispatched **plus** dead-lettered) — a worker loops on it and sleeps on `0`.
pub async fn drain_in_process_with<R, S, D>(
    reader: &R,
    registry: &SubscriberRegistry,
    inbox: &S,
    policy: RelayPolicy,
    dead_letters: &D,
) -> PortResult<usize>
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
    D: DeadLetterSink + Sync,
{
    let batch = reader.read_batch(policy.batch).await?;
    let mut handled = 0usize;
    for record in batch {
        // Retry this event in-pass, exactly as `dorm_outbox::relay` loops over its
        // attempts: a local counter, exponential backoff between tries, dead-letter
        // once the budget is spent. The cursor moves only when the event is settled
        // (dispatched or dead-lettered), so at-least-once holds and a poison pill is
        // buried instead of stalling the line.
        let mut attempt = 0u32;
        loop {
            match dispatch(&record.event, registry, inbox).await {
                Ok(()) => {
                    reader.advance(record.position).await?;
                    handled += 1;
                    break;
                }
                Err(err) => {
                    attempt += 1;
                    if attempt >= policy.max_attempts {
                        // Budget spent: record the poison pill and step over it so the
                        // consumer does not stall (the sink's own failure, by contrast,
                        // leaves the cursor put — see `DeadLetterSink::dead_letter`).
                        dead_letters.dead_letter(&record, &err.to_string()).await?;
                        reader.advance(record.position).await?;
                        handled += 1;
                        break;
                    }
                    tokio::time::sleep(policy.backoff(attempt)).await;
                }
            }
        }
    }
    Ok(handled)
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
///
/// This convenience applies [`RelayPolicy::default`] and [`TracingDeadLetters`]; use
/// [`run_in_process_dispatch_worker_with`] to tune the retry policy or route poison
/// pills to a durable dead-letter sink.
pub async fn run_in_process_dispatch_worker<R, S, Sig>(
    config: WorkerConfig,
    shutdown: Sig,
    reader: R,
    registry: SubscriberRegistry,
    inbox: S,
) -> WorkerReport
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
    Sig: Future<Output = ()>,
{
    run_in_process_dispatch_worker_with(
        config,
        shutdown,
        reader,
        registry,
        inbox,
        RelayPolicy::default(),
        TracingDeadLetters,
    )
    .await
}

/// [`run_in_process_dispatch_worker`] with an explicit failure [`policy`](RelayPolicy)
/// and [`dead_letters`](DeadLetterSink) sink, so a production monolith loops the
/// driver with the same retry budget as the broker relay and buries poison pills in
/// a durable store (e.g. the DORM `dorm_outbox_dead` adapter) rather than only
/// logging them.
pub async fn run_in_process_dispatch_worker_with<R, S, D, Sig>(
    config: WorkerConfig,
    shutdown: Sig,
    reader: R,
    registry: SubscriberRegistry,
    inbox: S,
    policy: RelayPolicy,
    dead_letters: D,
) -> WorkerReport
where
    R: OutboxReader + Sync,
    S: IdempotencyStore,
    D: DeadLetterSink + Sync,
    Sig: Future<Output = ()>,
{
    let reader = Arc::new(reader);
    let inbox = Arc::new(inbox);
    let registry = Arc::new(registry);
    let dead_letters = Arc::new(dead_letters);
    run_worker(config, shutdown, move || {
        // Each pass owns its clones, so the future borrows only its own locals and
        // stays `Send` across `.await` — the same shape as the relay worker.
        let reader = Arc::clone(&reader);
        let inbox = Arc::clone(&inbox);
        let registry = Arc::clone(&registry);
        let dead_letters = Arc::clone(&dead_letters);
        async move {
            let handled =
                drain_in_process_with(&*reader, &registry, &*inbox, policy, &*dead_letters).await?;
            Ok(handled as u64)
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

/// An in-memory [`DeadLetterSink`] double (behind the `testing` feature): it
/// captures every dead-lettered `(record, error)` so a test can assert a poison
/// pill was buried — the offline stand-in for the real `dorm_outbox_dead` table.
///
/// It is the dead-letter counterpart of [`InMemoryOutbox`]: a `Mutex`-guarded log,
/// nothing durable, enough to prove the driver's head-of-line-unblocking policy
/// (dead-letter + advance) in parity with `dorm_outbox::relay` with no database.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct InMemoryDeadLetters {
    inner: Arc<Mutex<Vec<(OutboxRecord, String)>>>,
}

#[cfg(any(test, feature = "testing"))]
impl InMemoryDeadLetters {
    /// An empty dead-letter log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every `(record, error)` dead-lettered so far, oldest first.
    #[must_use]
    pub fn captured(&self) -> Vec<(OutboxRecord, String)> {
        self.inner
            .lock()
            .expect("InMemoryDeadLetters poisoned")
            .clone()
    }

    /// How many events have been dead-lettered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("InMemoryDeadLetters poisoned")
            .len()
    }

    /// Whether nothing has been dead-lettered yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl DeadLetterSink for InMemoryDeadLetters {
    async fn dead_letter(&self, record: &OutboxRecord, error: &str) -> PortResult<()> {
        self.inner
            .lock()
            .expect("InMemoryDeadLetters poisoned")
            .push((record.clone(), error.to_owned()));
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
        // Inject the current trace context onto the envelope as it is written to the
        // outbox (ADR 0027), so the consumer can parent its handling span to the
        // emitting use case. A no-op (empty context) until a service opts into
        // tracing, so the offline build carries no trace attributes.
        let mut event = CloudEvent::new(id, &self.source, topic, data);
        let tc = trace_context::inject_current();
        if let Some(traceparent) = tc.traceparent {
            event = event.with_traceparent(traceparent);
        }
        if let Some(tracestate) = tc.tracestate {
            event = event.with_tracestate(tracestate);
        }
        self.outbox.publish(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_ports::{BoxError, EventPublisher};
    use seiva_testing::InMemoryIdempotencyStore;
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

    fn event(ty: &str, id: &str) -> CloudEvent {
        CloudEvent::new(id, "/shop/orders", ty, serde_json::Value::Null)
    }

    #[tokio::test]
    async fn one_event_runs_every_subscriber_of_its_type() {
        let inventory = CountingSubscriber::new("inventory");
        let shipping = CountingSubscriber::new("shipping");
        let mut registry = SubscriberRegistry::new();
        registry.register("orders.placed", Arc::new(inventory.clone()));
        registry.register("orders.placed", Arc::new(shipping.clone()));

        let inbox = InMemoryIdempotencyStore::new();
        dispatch(&event("orders.placed", "evt-1"), &registry, &inbox)
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
        let ev = event("orders.placed", "evt-1");
        dispatch(&ev, &registry, &inbox).await.unwrap();
        dispatch(&ev, &registry, &inbox).await.unwrap();

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
        )
        .await
        .expect_err("a failing subscriber must surface its error");
        assert!(err.to_string().contains("boom"));
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
}
