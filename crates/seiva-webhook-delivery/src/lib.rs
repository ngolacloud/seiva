//! Wire the ADR 0033 **webhook delivery engine** into the runtime.
//!
//! The engine in [`seiva_webhooks::delivery`] is deliberately runtime-free: it
//! composes a [`DeliveryLedger`], an [`HttpSender`] and a [`SigningSecrets`] into
//! one reliable send, but it neither reads an event bus nor spins a loop. This
//! crate supplies the two seams that close that gap — and nothing else:
//!
//! 1. **The enqueue side — a subscriber (ADR 0021).** [`DeliverySubscriber`] is a
//!    [`seiva_events::EventSubscriber`]: given one published domain event it
//!    resolves the matching [`Subscription`]s and writes one `pending`
//!    [`NewDelivery`] per subscription into the ledger. The enqueue happens
//!    **inside the handler**, so under the integrated driver (where a handler runs
//!    inside its `UnitOfWork::transaction` together with the inbox claim+mark) the
//!    delivery rows commit in the **same transaction** as the event is consumed —
//!    a committed event always has its deliveries owed, exactly like the
//!    idempotency reservation of ADR 0011. The [`DeliveryId`] is derived
//!    deterministically from `(subscription, event)` ([`delivery_id`]), so a
//!    redelivered event re-enqueues the *same* rows and the ledger's
//!    id-idempotent `enqueue` collapses them — no duplicate owed.
//!
//! 2. **The send side — a job (ADR 0034).** [`run_delivery_worker`] drives
//!    [`DeliveryEngine::tick`] on the shared [`seiva::worker`] loop: each pass
//!    claims a batch of due rows under a lease, delivers them, and records the
//!    outcome. Running on the worker loop means delivery is durable, leased and
//!    safe at N replicas by construction — this crate writes **no** new loop, only
//!    the one-pass [`delivery_tick`] the loop repeats.
//!
//! # What stays out
//!
//! This crate links **no HTTP client and no data layer**. The real
//! [`HttpSender`] adapter (reqwest/TLS) and a DORM-backed [`DeliveryLedger`] are
//! heavier, so they live in the crates excluded from the default workspace; this
//! crate stays a default member, DORM-free and fully offline-testable — its own
//! tests prove the enqueue side through the real [`seiva_events::dispatch`] path
//! with an in-memory ledger and inbox.

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use seiva::worker::{run_worker, WorkerConfig, WorkerReport};
use seiva_events::EventSubscriber;
use seiva_messaging::CloudEvent;
use seiva_ports::{BoxError, PortResult, Scope};
use seiva_webhooks::delivery::{
    DeliveryEngine, DeliveryId, DeliveryLedger, HttpSender, NewDelivery, SigningSecrets,
};

#[cfg(any(test, feature = "testing"))]
use std::collections::HashMap;

/// A single destination an event is delivered to: a tenant's registered endpoint.
///
/// It carries its own tenant [`Scope`] — **not an `Option`** — so a tenant-less
/// delivery is unrepresentable from this subscriber: every enqueued row is keyed
/// to a tenant (the ledger fail-closes on a missing scope as a backstop, ADR
/// 0019). "Which endpoints a tenant subscribed to which event" is the product's
/// policy, resolved through the [`Subscriptions`] port — this crate never owns a
/// subscription catalogue.
#[derive(Debug, Clone)]
pub struct Subscription {
    /// The subscription's stable id. It selects the signing secret
    /// ([`SigningSecrets::signer`]) and is half of the delivery id, so it must be
    /// stable across redeliveries.
    pub id: String,
    /// The destination URL. Egress-validated by the engine before every POST.
    pub endpoint: String,
    /// The tenant this subscription (and therefore the delivery) belongs to.
    pub scope: Scope,
}

/// Resolve the subscriptions a published event fans out to.
///
/// The product implements this over its own subscription catalogue (a table, a
/// config, a per-tenant policy); the subscriber asks it, once per event, "who
/// wants this?". Returning an empty list is a no-op (the event is owed to nobody).
#[async_trait]
pub trait Subscriptions: Send + Sync {
    /// The subscriptions interested in `event`, each of which gets one delivery.
    async fn matching(&self, event: &CloudEvent) -> PortResult<Vec<Subscription>>;
}

/// The stable delivery id for a `(subscription, event)` pair — it **is** the
/// `webhook-id` on the wire and therefore the receiver's idempotency key.
///
/// Deterministic on purpose: re-deriving it for the same pair yields the same id,
/// so a redelivered event re-enqueues the same rows (collapsed by the ledger's
/// id-idempotent `enqueue`) and a retried POST carries the same `webhook-id` the
/// receiver already saw (ADR 0011).
#[must_use]
pub fn delivery_id(subscription: &str, event_id: &str) -> DeliveryId {
    DeliveryId::new(format!("{subscription}.{event_id}"))
}

/// The enqueue side (ADR 0021): a [`seiva_events::EventSubscriber`] that turns one
/// published event into one owed delivery per matching [`Subscription`].
///
/// Generic over its two ports — the [`DeliveryLedger`] it writes to and the
/// [`Subscriptions`] it resolves against — so a test binds in-memory doubles and a
/// product binds a DORM-backed ledger plus its own catalogue, with no `dyn`
/// indirection inside the subscriber.
pub struct DeliverySubscriber<L, S> {
    id: String,
    ledger: L,
    subscriptions: S,
}

impl<L, S> DeliverySubscriber<L, S> {
    /// Build a subscriber under the stable `id` (its
    /// [`subscriber_id`](EventSubscriber::subscriber_id), which — with the event id
    /// — keys its inbox claim in [`dispatch`](seiva_events::dispatch), so it
    /// consumes each event once). `ledger` is where owed deliveries are written;
    /// `subscriptions` resolves who an event fans out to.
    pub fn new(id: impl Into<String>, ledger: L, subscriptions: S) -> Self {
        Self {
            id: id.into(),
            ledger,
            subscriptions,
        }
    }
}

#[async_trait]
impl<L, S> EventSubscriber for DeliverySubscriber<L, S>
where
    L: DeliveryLedger,
    S: Subscriptions,
{
    fn subscriber_id(&self) -> &str {
        &self.id
    }

    async fn handle(&self, event: &CloudEvent) -> PortResult<()> {
        // The signed-and-POSTed body is the CloudEvents envelope the subscriber
        // received — the SAME bytes the signature is computed over downstream, and
        // the same event the broker path would carry.
        let payload = serde_json::to_vec(&event.to_json())?;

        for subscription in self.subscriptions.matching(event).await? {
            let new = NewDelivery {
                // Deterministic: a redelivery re-enqueues the same id, which the
                // ledger collapses (id-idempotent), so one event is owed once.
                id: delivery_id(&subscription.id, &event.id),
                subscription: subscription.id,
                event_id: event.id.clone(),
                endpoint: subscription.endpoint,
                payload: payload.clone(),
                // The subscription's tenant — never absent (the ledger fail-closes
                // on `None` as a backstop, ADR 0019).
                scope: Some(subscription.scope),
            };
            let id = new.id.clone();
            // Enqueue INSIDE the handler, so under the integrated driver this write
            // commits in the same transaction as the event is consumed (ADR 0011).
            self.ledger
                .enqueue(new)
                .await
                .map_err(|error| Box::new(error) as BoxError)?;
            tracing::debug!(delivery = %id, event = %event.id, "delivery enqueued");
        }
        Ok(())
    }
}

/// One pass of the delivery **job** (ADR 0034): claim a batch of due rows and
/// deliver them, returning how many were claimed this pass (so the worker's
/// fast-drain path can empty a backlog). Maps the engine's error onto the worker's
/// [`PortResult`] boundary.
///
/// `SystemTime::now()` is read here — the job is the production edge where real
/// time enters; the engine itself takes `now` injected so its tests stay
/// deterministic.
///
/// # Errors
///
/// Returns the engine's [`DeliveryError`](seiva_webhooks::delivery::DeliveryError)
/// (boxed) only if the *claim* fails; a per-delivery record failure is tallied in
/// the tick's report, never surfaced here (so one poisoned row never fails the
/// whole pass).
pub async fn delivery_tick<L, S, K>(engine: &DeliveryEngine<L, S, K>) -> PortResult<u64>
where
    L: DeliveryLedger,
    S: HttpSender,
    K: SigningSecrets,
{
    let report = engine
        .tick(SystemTime::now())
        .await
        .map_err(|error| Box::new(error) as BoxError)?;
    Ok(report.claimed as u64)
}

/// Run the delivery [`job`](delivery_tick) in a loop until `shutdown` resolves —
/// the ADR 0034 job on the framework's shared [`seiva::worker`] loop, the SAME
/// loop the outbox relay and the in-process dispatcher use (interval, fast-drain,
/// graceful drain-and-exit, a [`WorkerReport`]). A failing pass is logged and
/// counted but never fatal.
///
/// This is how a service makes delivery progress on its own: a use case publishes
/// an event, the [`DeliverySubscriber`] records the owed deliveries, and this
/// worker drains them continuously — no hand-rolled retry loop. Enable the
/// fast-drain path ([`WorkerConfig::drain_fast_at`]) so a backlog empties without
/// idle waits.
pub async fn run_delivery_worker<L, S, K, Sig>(
    config: WorkerConfig,
    shutdown: Sig,
    engine: DeliveryEngine<L, S, K>,
) -> WorkerReport
where
    L: DeliveryLedger,
    S: HttpSender,
    K: SigningSecrets,
    Sig: Future<Output = ()>,
{
    let engine = Arc::new(engine);
    run_worker(config, shutdown, move || {
        let engine = Arc::clone(&engine);
        async move { delivery_tick(&engine).await }
    })
    .await
}

/// An in-memory [`Subscriptions`] double (behind the `testing` feature): a routing
/// table from CloudEvent `type` to the subscriptions interested in it, so the
/// enqueue side is proved OFFLINE with no catalogue and no database.
///
/// A product never uses this — it resolves subscriptions from its own per-tenant
/// catalogue; this is the stand-in a test registers a handful of subscriptions in.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Default)]
pub struct InMemorySubscriptions {
    by_type: HashMap<String, Vec<Subscription>>,
}

#[cfg(any(test, feature = "testing"))]
impl InMemorySubscriptions {
    /// An empty routing table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `subscription` as interested in `event_type` (the CloudEvent
    /// `type`). Additive: several subscriptions may share one type, and one event
    /// then fans out to each.
    pub fn register(&mut self, event_type: impl Into<String>, subscription: Subscription) {
        self.by_type
            .entry(event_type.into())
            .or_default()
            .push(subscription);
    }
}

#[cfg(any(test, feature = "testing"))]
#[async_trait]
impl Subscriptions for InMemorySubscriptions {
    async fn matching(&self, event: &CloudEvent) -> PortResult<Vec<Subscription>> {
        Ok(self.by_type.get(&event.ty).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_events::{dispatch, SubscriberRegistry};
    use seiva_testing::InMemoryIdempotencyStore;
    use seiva_webhooks::delivery::{DeliveryStatus, InMemoryLedger};

    const ENDPOINT_A: &str = "https://a.hooks.example.com/ingest";
    const ENDPOINT_B: &str = "https://b.hooks.example.com/ingest";

    fn tenant() -> Scope {
        Scope::tenant("acme").expect("a non-empty tenant")
    }

    fn event() -> CloudEvent {
        CloudEvent::new(
            "evt-1",
            "/shop/orders",
            "order.placed",
            serde_json::json!({ "order": 7 }),
        )
    }

    fn subscriptions() -> InMemorySubscriptions {
        let mut subs = InMemorySubscriptions::new();
        subs.register(
            "order.placed",
            Subscription {
                id: "sub_a".to_string(),
                endpoint: ENDPOINT_A.to_string(),
                scope: tenant(),
            },
        );
        subs.register(
            "order.placed",
            Subscription {
                id: "sub_b".to_string(),
                endpoint: ENDPOINT_B.to_string(),
                scope: tenant(),
            },
        );
        subs
    }

    #[tokio::test]
    async fn one_event_enqueues_one_row_per_matching_subscription() {
        let ledger = InMemoryLedger::new();
        let subscriber = DeliverySubscriber::new("webhooks", ledger.clone(), subscriptions());

        subscriber.handle(&event()).await.unwrap();

        assert_eq!(ledger.len(), 2, "one owed delivery per subscription");

        let a = ledger
            .get(&delivery_id("sub_a", "evt-1"))
            .await
            .unwrap()
            .expect("sub_a row");
        assert_eq!(a.endpoint, ENDPOINT_A);
        assert_eq!(a.subscription, "sub_a");
        assert_eq!(a.event_id, "evt-1");
        assert_eq!(a.status, DeliveryStatus::Pending);
        assert_eq!(a.attempt, 0);
        // The owed body is the CloudEvents envelope the subscriber received.
        let expected = serde_json::to_vec(&event().to_json()).unwrap();
        assert_eq!(a.payload, expected);
        // Every row is keyed to the subscription's tenant — no tenant-less send.
        assert_eq!(a.scope, tenant());

        let b = ledger
            .get(&delivery_id("sub_b", "evt-1"))
            .await
            .unwrap()
            .expect("sub_b row");
        assert_eq!(b.endpoint, ENDPOINT_B);
    }

    #[tokio::test]
    async fn redelivery_of_the_same_event_is_idempotent() {
        let ledger = InMemoryLedger::new();
        let subscriber = DeliverySubscriber::new("webhooks", ledger.clone(), subscriptions());

        // Handle the SAME event twice (a redelivery): the deterministic ids make
        // the second pass a no-op in the ledger — still one owed row per sub.
        subscriber.handle(&event()).await.unwrap();
        subscriber.handle(&event()).await.unwrap();

        assert_eq!(ledger.len(), 2, "a redelivery owes no duplicate");
    }

    #[tokio::test]
    async fn an_event_with_no_subscriptions_owes_nothing() {
        let ledger = InMemoryLedger::new();
        let subscriber =
            DeliverySubscriber::new("webhooks", ledger.clone(), InMemorySubscriptions::new());

        subscriber.handle(&event()).await.unwrap();

        assert!(ledger.is_empty(), "nobody subscribed ⇒ nothing owed");
    }

    #[tokio::test]
    async fn enqueues_through_the_real_dispatch_path() {
        // The subscriber plugs into the ADR 0021 bus: register it, then dispatch an
        // event through `seiva_events::dispatch` (the same entrypoint the in-process
        // driver and the broker drain call). The rows must land, and a redelivery
        // through the bus must still owe exactly one row per subscription — the
        // inbox dedups the effect and the ledger dedups the id.
        let ledger = InMemoryLedger::new();
        let subscriber = DeliverySubscriber::new("webhooks", ledger.clone(), subscriptions());

        let mut registry = SubscriberRegistry::new();
        registry.register("order.placed", Arc::new(subscriber));

        let inbox = InMemoryIdempotencyStore::new();
        let ev = event();
        dispatch(&ev, &registry, &inbox).await.unwrap();
        // A redelivery of the same event through the bus.
        dispatch(&ev, &registry, &inbox).await.unwrap();

        assert_eq!(
            ledger.len(),
            2,
            "two subscriptions ⇒ two owed rows, redelivery-safe"
        );
    }
}
