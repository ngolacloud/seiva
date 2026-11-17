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
//! The three guarantees ADR 0021 demands of the in-process path are proved:
//!
//! - **(a)** one `orders.placed` reserves inventory exactly once;
//! - **(b)** running the driver again over the drained outbox reserves nothing more
//!   — proved both by the **cursor** (nothing left to read) and, under a forced
//!   at-least-once **redelivery**, by the **inbox** (the effect is deduped);
//! - **(c)** an `orders.placed` whose subscriber *recovers* within the retry budget
//!   still runs its effect exactly once (in-pass retry), while one that fails
//!   *persistently* is dead-lettered after `max_attempts` and the cursor advances
//!   past it — the same max-attempts + backoff + dead-letter policy the broker relay
//!   applies, so a poison pill never blocks the head of the line (ADR 0028, D-2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use seiva_events::{
    drain_in_process, drain_in_process_with, EventSubscriber, InMemoryDeadLetters, InMemoryOutbox,
    RelayPolicy, SubscriberRegistry,
};
use seiva_messaging::CloudEvent;
use seiva_ports::PortResult;
use seiva_testing::InMemoryIdempotencyStore;

/// A fast, deterministic policy for the failure tests: no backoff sleeps, a small
/// retry budget. Mirrors `dorm_outbox::RelayConfig` field-for-field — the point of
/// D-2 is that in-process and broker share **one** policy type.
fn fast_policy(max_attempts: u32) -> RelayPolicy {
    RelayPolicy {
        max_attempts,
        backoff_base_ms: 0,
        ..RelayPolicy::default()
    }
}

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

    // The `orders` module publishes its domain event into the (in-memory) outbox.
    let outbox = InMemoryOutbox::new();
    outbox.publish(orders_placed("order-1"));

    // (a) One pass dispatches the event; inventory reserves exactly once.
    let n = drain_in_process(&outbox, &registry, &inbox).await.unwrap();
    assert_eq!(n, 1, "one event was dispatched");
    assert_eq!(inventory.reserved(), 1, "inventory reserved once");
    assert_eq!(
        outbox.pending(),
        0,
        "the cursor advanced past the drained event"
    );

    // (b) cursor: draining the already-drained outbox reserves nothing more.
    let n = drain_in_process(&outbox, &registry, &inbox).await.unwrap();
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
    let n = drain_in_process(&outbox, &registry, &inbox).await.unwrap();
    assert_eq!(n, 1, "the event was re-dispatched");
    assert_eq!(
        inventory.reserved(),
        1,
        "but the inbox kept the effect exactly-once under redelivery"
    );
}

#[tokio::test]
async fn a_persistently_failing_subscriber_is_dead_lettered_and_the_cursor_advances() {
    // (c) / D-2: a subscriber that fails *persistently* must not block the head of
    // the line forever. After `max_attempts` the driver dead-letters the event and
    // advances the cursor past it — the SAME policy `dorm_outbox::relay` applies on
    // the broker path, so `module split` does not change the delivery semantics. A
    // second, healthy event sits BEHIND the poison pill to prove the dead-letter
    // UNBLOCKS the line (under the old head-of-line behaviour it would starve
    // forever).
    struct PoisonForOrder1 {
        attempts: Arc<AtomicU64>,
        reserved: Arc<AtomicU64>,
    }

    #[async_trait]
    impl EventSubscriber for PoisonForOrder1 {
        fn subscriber_id(&self) -> &str {
            "inventory"
        }

        async fn handle(&self, event: &CloudEvent) -> PortResult<()> {
            // `order-1` is the poison pill — it never succeeds; `order-2` reserves.
            if event.id == "order-1" {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                return Err("stock service unavailable".into());
            }
            self.reserved.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let attempts = Arc::new(AtomicU64::new(0));
    let reserved = Arc::new(AtomicU64::new(0));
    let mut registry = SubscriberRegistry::new();
    registry.register(
        "orders.placed",
        Arc::new(PoisonForOrder1 {
            attempts: attempts.clone(),
            reserved: reserved.clone(),
        }),
    );
    let inbox = InMemoryIdempotencyStore::new();

    let outbox = InMemoryOutbox::new();
    outbox.publish(orders_placed("order-1")); // poison pill, oldest in the cursor
    outbox.publish(orders_placed("order-2")); // healthy, behind it

    let policy = fast_policy(3);
    let dead = InMemoryDeadLetters::new();
    let handled = drain_in_process_with(&outbox, &registry, &inbox, policy, &dead)
        .await
        .expect("the pass itself succeeds — the failure is dead-lettered, not propagated");

    assert_eq!(
        attempts.load(Ordering::SeqCst),
        u64::from(policy.max_attempts),
        "the poison pill was retried exactly max_attempts times before giving up"
    );
    assert_eq!(
        handled, 2,
        "both events were settled this pass (one dead-lettered, one dispatched)"
    );
    assert_eq!(dead.len(), 1, "exactly the poison pill was dead-lettered");
    let (record, error) = dead.captured().pop().unwrap();
    assert_eq!(
        record.event.id, "order-1",
        "the dead-lettered event is the poison one"
    );
    assert!(
        error.contains("stock service unavailable"),
        "the last error is carried into the dead-letter: {error}"
    );
    assert_eq!(
        reserved.load(Ordering::SeqCst),
        1,
        "the healthy event behind the poison pill WAS delivered — the line unblocked"
    );
    assert_eq!(
        outbox.pending(),
        0,
        "the cursor advanced past both events; nothing is stuck"
    );
}

#[tokio::test]
async fn a_transient_subscriber_failure_still_runs_the_effect_on_retry() {
    // D-1 / ADR 0028, under the D-2 in-pass retry: a subscriber that fails
    // *transiently* on its first attempt and succeeds on the retry must still run
    // its effect exactly once — WITHIN a single pass now, because the driver retries
    // in-pass (as the broker relay does) up to `max_attempts`. The seam that makes
    // this work is `IdempotencyStore::abort`: on each failed attempt the claim is
    // released, so the next attempt is `Fresh` again and the effect finally runs
    // (without it, the retry would see an orphaned `InProgress` and skip the handler,
    // losing the effect). The event is NOT dead-lettered, since it recovers inside
    // the budget.
    struct FlakyOnce {
        attempts: Arc<AtomicU64>,
        effects: Arc<AtomicU64>,
    }

    #[async_trait]
    impl EventSubscriber for FlakyOnce {
        fn subscriber_id(&self) -> &str {
            "inventory"
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            // Fail only the first attempt, then succeed and record the effect.
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err("stock service briefly unavailable".into());
            }
            self.effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let attempts = Arc::new(AtomicU64::new(0));
    let effects = Arc::new(AtomicU64::new(0));
    let mut registry = SubscriberRegistry::new();
    registry.register(
        "orders.placed",
        Arc::new(FlakyOnce {
            attempts: attempts.clone(),
            effects: effects.clone(),
        }),
    );
    let inbox = InMemoryIdempotencyStore::new();

    let outbox = InMemoryOutbox::new();
    outbox.publish(orders_placed("order-1"));

    // One pass: attempt 1 fails (claim aborted), attempt 2 succeeds and runs the
    // effect — all in-pass, no dead-letter, cursor advanced.
    let dead = InMemoryDeadLetters::new();
    let handled = drain_in_process_with(&outbox, &registry, &inbox, fast_policy(5), &dead)
        .await
        .unwrap();

    assert_eq!(handled, 1, "the event was settled (dispatched) this pass");
    assert_eq!(
        effects.load(Ordering::SeqCst),
        1,
        "the effect ran exactly once, on the successful retry (lost without `abort`)"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "the handler was retried once, in-pass"
    );
    assert!(dead.is_empty(), "a recovered event is NOT dead-lettered");
    assert_eq!(outbox.pending(), 0, "the cursor advanced after the success");
}
