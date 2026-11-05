//! In-memory reference doubles for the core ports (ADR 0002).
//!
//! One double per port in [`seiva_ports`]. Construct a use case with these and
//! assert on recorded events and transaction outcomes, with no server, broker
//! or database involved. Each double is cheap to clone and shares its recorded
//! state, so a test can hold a handle, pass a clone into the handler, and read
//! back what happened.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use seiva_ports::{
    Clock, EventPublisher, Id, IdGenerator, IdempotencyOutcome, IdempotencyStore, PortResult,
    UnitOfWork,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A [`Clock`] fixed at a chosen instant, advanceable for time-based tests.
#[derive(Clone)]
pub struct FixedClock {
    now: Arc<Mutex<SystemTime>>,
}

impl FixedClock {
    /// A clock fixed at `t`.
    pub fn at(t: SystemTime) -> Self {
        Self {
            now: Arc::new(Mutex::new(t)),
        }
    }

    /// A clock fixed at `secs` seconds after the Unix epoch.
    pub fn at_unix_secs(secs: u64) -> Self {
        Self::at(UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().expect("FixedClock poisoned");
        *now += by;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        *self.now.lock().expect("FixedClock poisoned")
    }
}

/// An [`IdGenerator`] that hands out deterministic ids from a counter, written
/// big-endian into the 16 bytes of [`Id`].
#[derive(Clone)]
pub struct SequenceIdGenerator {
    next: Arc<Mutex<u128>>,
}

impl SequenceIdGenerator {
    /// A generator whose first id is `1`.
    pub fn new() -> Self {
        Self::starting_at(1)
    }

    /// A generator whose first id is `start`.
    pub fn starting_at(start: u128) -> Self {
        Self {
            next: Arc::new(Mutex::new(start)),
        }
    }
}

impl Default for SequenceIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdGenerator for SequenceIdGenerator {
    fn new_id(&self) -> Id {
        let mut next = self.next.lock().expect("SequenceIdGenerator poisoned");
        let value = *next;
        *next += 1;
        Id(value.to_be_bytes())
    }
}

/// One event captured by [`RecordingEventPublisher`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedEvent {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// An [`EventPublisher`] that records every published event for assertions.
#[derive(Clone, Default)]
pub struct RecordingEventPublisher {
    events: Arc<Mutex<Vec<RecordedEvent>>>,
}

impl RecordingEventPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event published so far, in order.
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events.lock().expect("publisher poisoned").clone()
    }

    /// The topics published so far, in order.
    pub fn topics(&self) -> Vec<String> {
        self.events().into_iter().map(|event| event.topic).collect()
    }

    /// How many events were published.
    pub fn count(&self) -> usize {
        self.events.lock().expect("publisher poisoned").len()
    }
}

#[async_trait]
impl EventPublisher for RecordingEventPublisher {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()> {
        self.events
            .lock()
            .expect("publisher poisoned")
            .push(RecordedEvent {
                topic: topic.to_owned(),
                payload: payload.to_owned(),
            });
        Ok(())
    }
}

/// What happened to a transaction run through [`InMemoryUnitOfWork`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxOutcome {
    Committed,
    RolledBack,
}

/// A scope-based [`UnitOfWork`] that records each transaction's outcome: the
/// work body committing (`Ok`) or rolling back (`Err`), mirroring the guarantee
/// the DORM-backed one keeps. Hold a clone, pass one into the handler, then read
/// [`outcomes`](Self::outcomes) back after the use case runs.
#[derive(Clone, Default)]
pub struct InMemoryUnitOfWork {
    log: Arc<Mutex<Vec<TxOutcome>>>,
}

impl InMemoryUnitOfWork {
    pub fn new() -> Self {
        Self::default()
    }

    /// The outcome of every transaction run so far, in order.
    pub fn outcomes(&self) -> Vec<TxOutcome> {
        self.log.lock().expect("uow poisoned").clone()
    }
}

impl UnitOfWork for InMemoryUnitOfWork {
    fn transaction<'a, T, F, Fut>(
        &'a self,
        work: F,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PortResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: std::future::Future<Output = PortResult<T>> + Send + 'a,
        T: Send + 'a,
    {
        Box::pin(async move {
            match work().await {
                Ok(value) => {
                    self.log
                        .lock()
                        .expect("uow poisoned")
                        .push(TxOutcome::Committed);
                    Ok(value)
                }
                Err(err) => {
                    self.log
                        .lock()
                        .expect("uow poisoned")
                        .push(TxOutcome::RolledBack);
                    Err(err)
                }
            }
        })
    }
}

/// An in-memory [`IdempotencyStore`] double for unit-testing the idempotency
/// **state machine** (Fresh → Replay, Mismatch, InProgress) with no database.
///
/// It is a faithful single-process model: the `Mutex` serialises `begin`, so a
/// key claimed and not yet `finish`ed answers [`IdempotencyOutcome::InProgress`]
/// to a second claim, and a completed key answers [`Replay`](IdempotencyOutcome::Replay).
/// It does **not** prove the *distributed, durable* guarantee — a Mutex is not a
/// cross-replica CAS and nothing survives a restart. The real guarantee is proved
/// against a live database with the DORM-backed store (ADR 0011); use this only
/// for logic tests.
#[derive(Clone, Default)]
pub struct InMemoryIdempotencyStore {
    keys: Arc<Mutex<HashMap<String, IdemRecord>>>,
}

#[derive(Clone)]
struct IdemRecord {
    fingerprint: String,
    response: Option<Vec<u8>>,
}

impl InMemoryIdempotencyStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn begin(
        &self,
        scoped_key: &str,
        fingerprint: &str,
        _ttl: Duration,
    ) -> PortResult<IdempotencyOutcome> {
        let mut keys = self.keys.lock().expect("idempotency store poisoned");
        match keys.get(scoped_key) {
            None => {
                keys.insert(
                    scoped_key.to_owned(),
                    IdemRecord {
                        fingerprint: fingerprint.to_owned(),
                        response: None,
                    },
                );
                Ok(IdempotencyOutcome::Fresh)
            }
            Some(record) if record.fingerprint != fingerprint => Ok(IdempotencyOutcome::Mismatch),
            Some(record) => match &record.response {
                Some(bytes) => Ok(IdempotencyOutcome::Replay(bytes.clone())),
                None => Ok(IdempotencyOutcome::InProgress),
            },
        }
    }

    async fn finish(&self, scoped_key: &str, response: &[u8]) -> PortResult<()> {
        let mut keys = self.keys.lock().expect("idempotency store poisoned");
        if let Some(record) = keys.get_mut(scoped_key) {
            record.response = Some(response.to_vec());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_holds_and_advances() {
        let clock = FixedClock::at_unix_secs(1_000);
        assert_eq!(clock.now(), UNIX_EPOCH + Duration::from_secs(1_000));
        clock.advance(Duration::from_secs(5));
        assert_eq!(clock.now(), UNIX_EPOCH + Duration::from_secs(1_005));
    }

    #[test]
    fn sequence_ids_are_deterministic_and_distinct() {
        let gen = SequenceIdGenerator::new();
        assert_eq!(gen.new_id(), Id(1u128.to_be_bytes()));
        assert_eq!(gen.new_id(), Id(2u128.to_be_bytes()));
        let gen2 = SequenceIdGenerator::starting_at(10);
        assert_eq!(gen2.new_id(), Id(10u128.to_be_bytes()));
    }

    #[tokio::test]
    async fn recording_publisher_captures_events_in_order() {
        let publisher = RecordingEventPublisher::new();
        publisher.publish("notes.created", b"a").await.unwrap();
        publisher.publish("notes.updated", b"b").await.unwrap();
        assert_eq!(publisher.count(), 2);
        assert_eq!(publisher.topics(), ["notes.created", "notes.updated"]);
        assert_eq!(publisher.events()[0].payload, b"a");
    }

    #[tokio::test]
    async fn an_ok_body_commits_and_an_err_body_rolls_back() {
        let uow = InMemoryUnitOfWork::new();
        let ok: PortResult<i32> = uow.transaction(|| async { Ok(7) }).await;
        assert_eq!(ok.unwrap(), 7);
        let err: PortResult<i32> = uow.transaction(|| async { Err("boom".into()) }).await;
        assert!(err.is_err());
        assert_eq!(
            uow.outcomes(),
            [TxOutcome::Committed, TxOutcome::RolledBack]
        );
    }

    // The payoff of ADR 0002: a use case tested with the four doubles alone —
    // no server, no broker, no database. This is the shape of a real module's
    // application-layer unit test. The handler depends only on the ports.
    struct CreateNote<C: Clock, G: IdGenerator, U: UnitOfWork, E: EventPublisher> {
        clock: C,
        ids: G,
        uow: U,
        events: E,
    }

    impl<C: Clock, G: IdGenerator, U: UnitOfWork, E: EventPublisher> CreateNote<C, G, U, E> {
        /// Mints an id inside a transaction, then publishes the created event
        /// only after the transaction commits. Returns the new note's id and time.
        async fn execute(&self) -> PortResult<(Id, SystemTime)> {
            let created_at = self.clock.now();
            // The transaction commits iff the body returns Ok (scope-based UoW).
            let id = self
                .uow
                .transaction(|| async { Ok(self.ids.new_id()) })
                .await?;
            // Publish after a successful commit (the relay draining the outbox);
            // the id bytes stand in for a real serialized event.
            self.events.publish("notes.created", &id.0).await?;
            Ok((id, created_at))
        }
    }

    #[tokio::test]
    async fn use_case_runs_against_the_four_doubles_only() {
        let handler = CreateNote {
            clock: FixedClock::at_unix_secs(1_700_000_000),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        };

        let (id, created_at) = handler.execute().await.unwrap();

        assert_eq!(id, Id(1u128.to_be_bytes()));
        assert_eq!(created_at, UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        // The event is visible only because the transaction committed first.
        assert_eq!(handler.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(handler.events.topics(), ["notes.created"]);
        assert_eq!(handler.events.events()[0].payload, id.0);
    }

    #[tokio::test]
    async fn idempotency_state_machine_fresh_then_replay_mismatch_inprogress() {
        let store = InMemoryIdempotencyStore::new();
        let ttl = Duration::from_secs(60);

        // First claim is Fresh.
        assert_eq!(
            store.begin("acme:create:k1", "fp-a", ttl).await.unwrap(),
            IdempotencyOutcome::Fresh
        );
        // While in flight (no finish yet), a second claim is InProgress.
        assert_eq!(
            store.begin("acme:create:k1", "fp-a", ttl).await.unwrap(),
            IdempotencyOutcome::InProgress
        );
        // The same key with a DIFFERENT fingerprint is a Mismatch (never executes).
        assert_eq!(
            store.begin("acme:create:k1", "fp-b", ttl).await.unwrap(),
            IdempotencyOutcome::Mismatch
        );
        // After finishing, the key replays the stored response.
        store
            .finish("acme:create:k1", b"stored-result")
            .await
            .unwrap();
        assert_eq!(
            store.begin("acme:create:k1", "fp-a", ttl).await.unwrap(),
            IdempotencyOutcome::Replay(b"stored-result".to_vec())
        );
        // A different scoped key (another tenant) is wholly independent.
        assert_eq!(
            store.begin("globex:create:k1", "fp-a", ttl).await.unwrap(),
            IdempotencyOutcome::Fresh
        );
    }

    #[tokio::test]
    async fn inbox_dedup_consumes_a_redelivered_event_once() {
        use seiva_ports::inbox_key;
        let store = InMemoryIdempotencyStore::new();
        let ttl = Duration::from_secs(60);
        let key = inbox_key("billing", "evt-1");
        let mut effects = 0;

        // Two deliveries of the same (consumer, event): claim+mark, effect once.
        for _ in 0..2 {
            if store.begin(&key, "evt-1", ttl).await.unwrap() == IdempotencyOutcome::Fresh {
                effects += 1;
                store.finish(&key, b"").await.unwrap();
            }
        }
        assert_eq!(effects, 1, "a redelivered event is consumed once");

        // A different event id for the same consumer is independent.
        assert_eq!(
            store
                .begin(&inbox_key("billing", "evt-2"), "evt-2", ttl)
                .await
                .unwrap(),
            IdempotencyOutcome::Fresh
        );
    }
}
