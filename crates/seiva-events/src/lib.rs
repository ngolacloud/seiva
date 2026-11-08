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
//! ## What is here (Fase 1) and what is not (Fase 2)
//!
//! This crate is **DORM-free and broker-free**, so the choreography is testable
//! fully offline. It provides the dispatch core only:
//!
//! - [`EventSubscriber`] — the handler a module plugs in for an event type;
//! - [`SubscriberRegistry`] — the composition-root routing table, by type;
//! - [`dispatch`] — deliver one event to its subscribers, deduped per subscriber.
//!
//! It deliberately does **not** read the real outbox (that is DORM's, through
//! `dorm-outbox`) nor run the worker loop that polls it. Wiring this dispatcher
//! to the outbox — the DORM-coupled driver that makes `module split` a swap of
//! dispatcher for `seiva-outbox-relay` — is **Fase 2** and lands alongside
//! `seiva-outbox-relay` (which is excluded from the default workspace because it
//! is DORM-git-coupled). The `IdempotencyStore` the real driver passes will be
//! the DORM-backed one, and the claim + effect + mark will share one
//! transaction; here any `IdempotencyStore` double proves the logic.
//!
//! [`seiva-outbox-relay`]: https://github.com/ngolacloud/seiva
//! [`CloudEvent`]: seiva_messaging::CloudEvent
//! [`inbox_key`]: seiva_ports::inbox_key
//! [`IdempotencyStore`]: seiva_ports::IdempotencyStore

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use seiva_messaging::CloudEvent;
use seiva_ports::{inbox_key, IdempotencyOutcome, IdempotencyStore, PortResult};

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
    for subscriber in registry.subscribers_for(&event.ty) {
        let key = inbox_key(subscriber.subscriber_id(), &event.id);
        // The event id is the fingerprint: the same event always claims the same
        // key with the same fingerprint, so a redelivery can never be a Mismatch.
        match inbox.begin(&key, &event.id, INBOX_TTL).await? {
            IdempotencyOutcome::Fresh => {
                // Order mirrors the broker consume path (claim → effect → mark);
                // in Fase 2 these commit in one transaction.
                subscriber.handle(event).await?;
                inbox.finish(&key, &[]).await?;
            }
            // Already consumed by this subscriber — a redelivery, no effect.
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

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_ports::BoxError;
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
}
