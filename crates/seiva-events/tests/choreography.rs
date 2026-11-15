//! Proof of the choreography `orders.placed → inventory.reserved`, fully offline
//! (ADR 0021, Fase 2): no broker, no DORM, no database.
//!
//! The `orders` module publishes an `orders.placed` event into an in-memory outbox
//! ([`InMemoryOutbox`]); a locally-registered `inventory` subscriber "reserves
//! stock" by bumping a counter; and the in-process driver ([`drain_in_process`])
//! drains the outbox and dispatches each event. The subscriber names no module
//! crate — the composition root (here, the test) routes the event type to it — which
//! is exactly the independence that lets `seiva module split` be a composition change.
//!
//! The guarantees ADR 0021 demands of the in-process path are proved:
//!
//! - **(a)** one `orders.placed` reserves inventory exactly once;
//! - **(b)** running the driver again over the drained outbox reserves nothing more
//!   — proved both by the **cursor** (nothing left to read) and, under a forced
//!   at-least-once **redelivery**, by the **inbox** (the effect is deduped);
//! - **(c)** an `orders.placed` whose subscriber always fails is retried the policy's
//!   budget and then **dead-lettered**, so the cursor advances past it rather than
//!   the poison event blocking the queue (ADR 0021 Fase 2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use seiva_events::{
    drain_in_process, EventSubscriber, InMemoryDeadLetterQueue, InMemoryOutbox, RetryPolicy,
    SubscriberRegistry,
};
use seiva_messaging::CloudEvent;
use seiva_ports::PortResult;
use seiva_testing::{InMemoryIdempotencyStore, InMemoryUnitOfWork};

/// A test-local stand-in for the `inventory` module: on `orders.placed` it "reserves
/// stock" by incrementing a counter — the observable **effect** we assert happens
/// exactly once. It imports no `orders`/`inventory` crate; the registry routes the
/// event to it.
#[derive(Clone)]
struct ReservingInventory {
    reserved: Arc<AtomicU64>,
}

impl ReservingInventory {
    fn new() -> Self {
        Self {
            reserved: Arc::new(AtomicU64::new(0)),
        }
    }

    fn reserved(&self) -> u64 {
        self.reserved.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EventSubscriber for ReservingInventory {
    fn subscriber_id(&self) -> &str {
        "inventory"
    }

    async fn handle(&self, event: &CloudEvent) -> PortResult<()> {
        assert_eq!(event.ty, "orders.placed", "routed only its own event type");
        self.reserved.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The event the `orders` module publishes on checkout.
fn orders_placed(id: &str) -> CloudEvent {
    CloudEvent::new(
        id,
        "/shop/orders",
        "orders.placed",
        serde_json::json!({ "order_id": id, "product_id": "42", "qty": 3 }),
    )
}

#[tokio::test]
async fn orders_placed_reserves_inventory_exactly_once_across_redelivery() {
    let inventory = ReservingInventory::new();
    let mut registry = SubscriberRegistry::new();
    registry.register("orders.placed", Arc::new(inventory.clone()));
    // One shared inbox across every pass — the dedup authority (ADR 0011).
    let inbox = InMemoryIdempotencyStore::new();
    let uow = InMemoryUnitOfWork::new();
    let policy = RetryPolicy::no_retry();
    let dlq = InMemoryDeadLetterQueue::new();

    // The `orders` module publishes its domain event into the (in-memory) outbox.
    let outbox = InMemoryOutbox::new();
    outbox.publish(orders_placed("order-1"));

    // (a) One pass dispatches the event; inventory reserves exactly once.
    let n = drain_in_process(&outbox, &registry, &inbox, &uow, &policy, &dlq)
        .await
        .unwrap();
    assert_eq!(n, 1, "one event was dispatched");
    assert_eq!(inventory.reserved(), 1, "inventory reserved once");
    assert_eq!(
        outbox.pending(),
        0,
        "the cursor advanced past the drained event"
    );

    // (b) cursor: draining the already-drained outbox reserves nothing more.
    let n = drain_in_process(&outbox, &registry, &inbox, &uow, &policy, &dlq)
        .await
        .unwrap();
    assert_eq!(n, 0, "nothing left past the cursor");
    assert_eq!(inventory.reserved(), 1, "still exactly one reservation");

    // (b') inbox: a genuine at-least-once REDELIVERY (crash after dispatch, before
    // the cursor advanced) is re-read and dispatched again, yet reserves nothing
    // more — the inbox absorbs it independently of the cursor.
    outbox.redeliver(0);
    assert_eq!(
        outbox.pending(),
        1,
        "the redelivered event is readable again"
    );
    let n = drain_in_process(&outbox, &registry, &inbox, &uow, &policy, &dlq)
        .await
        .unwrap();
    assert_eq!(n, 1, "the event was re-dispatched");
    assert_eq!(
        inventory.reserved(),
        1,
        "but the inbox kept the effect exactly-once under redelivery"
    );
    assert!(
        dlq.is_empty(),
        "nothing was dead-lettered on the happy path"
    );
}

#[tokio::test]
async fn an_always_failing_subscriber_is_retried_then_dead_lettered() {
    // (c) A subscriber whose reservation always fails: the driver retries it the
    // policy's budget, then dead-letters the event and advances the cursor — the
    // poison event is quarantined instead of blocking the queue forever (Fase 2).
    struct FlakyInventory;

    #[async_trait]
    impl EventSubscriber for FlakyInventory {
        fn subscriber_id(&self) -> &str {
            "inventory"
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            Err("stock service unavailable".into())
        }
    }

    let mut registry = SubscriberRegistry::new();
    registry.register("orders.placed", Arc::new(FlakyInventory));
    let inbox = InMemoryIdempotencyStore::new();
    let uow = InMemoryUnitOfWork::new();
    let dlq = InMemoryDeadLetterQueue::new();
    // Three attempts, immediate (no sleep) so the test is fast.
    let policy = RetryPolicy::new(3, Duration::ZERO);

    let outbox = InMemoryOutbox::new();
    outbox.publish(orders_placed("order-1"));

    let n = drain_in_process(&outbox, &registry, &inbox, &uow, &policy, &dlq)
        .await
        .expect("the poison event is dead-lettered, not propagated");
    assert_eq!(n, 1, "the event left the outbox (dead-lettered)");
    assert_eq!(
        outbox.pending(),
        0,
        "the cursor advanced past the poison event so it cannot block the queue"
    );
    assert_eq!(dlq.len(), 1, "the poison event was quarantined");
    assert_eq!(dlq.entries()[0].attempts, 3, "it was tried the full budget");
}
