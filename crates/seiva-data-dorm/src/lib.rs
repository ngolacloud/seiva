//! The DORM data-layer gateway (Parte C; Seiva ADR 0004).
//!
//! All coupling to DORM lives here. The rest of Seiva depends only on
//! `seiva-ports`; a module's `infrastructure` crate uses this gateway to build
//! its DORM handle and to satisfy the ports its `application` declared. Turning
//! the DORM data layer off is simply not compiling this crate (the brief's Parte
//! C), which is why the coupling is confined to one place.
//!
//! It compiles against DORM pinned by git revision and implements **all four
//! ports**: [`Clock`], [`IdGenerator`], the scope-based [`UnitOfWork`] via
//! [`DormUnitOfWork`] (which maps straight onto DORM's `with_session` +
//! `db::atomic` — the resolution ADR 0004 chose for the tx-model mismatch), and
//! [`EventPublisher`] via [`DormOutboxPublisher`] (writes to the transactional
//! outbox in the current transaction; the relay worker forwards it afterwards).
//! [`connect`] builds a module's `Database` while enforcing the two boundary
//! invariants a framework must not forget — strict sessions on (no silent global
//! `DEFAULT_DB`) and the keyset cursor key configured (no forgeable default).

#![forbid(unsafe_code)]

use async_trait::async_trait;
use seiva_messaging::CloudEvent;
use seiva_ports::{BoxError, Clock, EventPublisher, Id, IdGenerator, PortResult, UnitOfWork};
use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;

/// The real system clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A UUIDv4-backed id generator. The 16 random bytes fill [`Id`] directly, so
/// the framework and adapters render it as a UUID.
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn new_id(&self) -> Id {
        Id(*uuid::Uuid::new_v4().as_bytes())
    }
}

/// The data-layer configuration a framework hands the gateway, mapped onto
/// DORM's own [`dorm_core::db::ConnectOptions`]. Per module: its own DSN,
/// replicas, PostgreSQL schema and credentials.
#[derive(Debug, Clone)]
pub struct DormConfig {
    /// The primary DSN.
    pub dsn: String,
    /// Read-replica DSNs, if any.
    pub replicas: Vec<String>,
    /// The default read-only flag for sessions on this handle.
    pub read_only: bool,
    /// The PostgreSQL schema (namespace) this module's handle is bound to.
    pub schema: Option<String>,
    /// Maximum pooled connections.
    pub pool_max: usize,
    /// The HMAC key for opaque keyset cursors (from the framework's secrets).
    pub cursor_key: Vec<u8>,
}

/// Builds a module's DORM [`Database`](dorm_core::db::Database) from `config`,
/// and enforces the boundary invariants DORM-B1 established: **strict sessions**
/// (a statement with no session in scope is an error, never the process-global
/// default) and the **cursor signing key** (no forgeable default). The returned
/// handle is driven per request through `dorm_core::db::with_session`.
pub fn connect(config: &DormConfig) -> Result<dorm_core::db::Database, dorm_core::Error> {
    // A framework always runs through explicit per-module, per-request sessions.
    dorm_core::db::set_strict_sessions(true);
    dorm_core::cursor::set_cursor_key(config.cursor_key.clone());

    let options = dorm_core::db::ConnectOptions {
        pool_max: config.pool_max,
        ..dorm_core::db::ConnectOptions::default()
    };
    let replicas: Vec<&str> = config.replicas.iter().map(String::as_str).collect();
    dorm_core::db::Database::connect_with(
        &config.dsn,
        &replicas,
        config.read_only,
        config.schema.as_deref(),
        &options,
    )
}

/// The DORM-backed [`UnitOfWork`] (ADR 0004): the scope-based port mapped onto
/// DORM's closure-scoped executor. `transaction` runs the body inside the
/// module's session (`with_session` — strict mode requires one) and a DORM
/// transaction (`db::atomic`), which commits when the body returns `Ok` and
/// rolls back when it returns `Err`. The internal error is wrapped as the
/// boundary's [`BoxError`]; this is the bridge the ADR-0004 finding resolves.
pub struct DormUnitOfWork {
    db: Arc<dorm_core::db::Database>,
}

impl DormUnitOfWork {
    /// Wraps a module's [`connect`]-built handle as a unit of work.
    pub fn new(db: dorm_core::db::Database) -> Self {
        Self { db: Arc::new(db) }
    }
}

impl UnitOfWork for DormUnitOfWork {
    async fn transaction<T, F, Fut>(&self, work: F) -> PortResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = PortResult<T>> + Send,
        T: Send,
    {
        let db = self.db.clone();
        let outcome: Result<T, dorm_core::Error> = dorm_core::db::with_session(db, async move {
            dorm_core::db::atomic(move |_tx| async move {
                // Map the boundary error to a DORM error so `atomic` rolls back;
                // the message is preserved across the bridge.
                work()
                    .await
                    .map_err(|e| dorm_core::Error::Config(e.to_string()))
            })
            .await
        })
        .await;
        outcome.map_err(|e| Box::new(e) as BoxError)
    }
}

/// The DORM-backed [`EventPublisher`]: writes the event to the transactional
/// outbox (`dorm-outbox`) **in the current transaction**. Paired with
/// [`DormUnitOfWork`], the event and the business change commit together (the
/// transactional-outbox pattern) — so it must be called inside a
/// [`UnitOfWork::transaction`]; `dorm-outbox` refuses a publish outside one. The
/// relay worker (`seiva-outbox-relay`) forwards the events to the broker
/// afterwards. The payload is the serialized event as JSON (the interface's
/// CloudEvents envelope).
pub struct DormOutboxPublisher;

#[async_trait]
impl EventPublisher for DormOutboxPublisher {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()> {
        // The application emits the event's DATA as JSON (it knows nothing of
        // CloudEvents). The gateway wraps it in a CloudEvent envelope — the shape
        // the relay reconstructs with `CloudEvent::from_json` before forwarding —
        // and stores that in the outbox. The id is a fresh UUID; the source marks
        // the framework data layer; the CloudEvents `type` is the topic.
        let data: serde_json::Value =
            serde_json::from_slice(payload).map_err(|e| Box::new(e) as BoxError)?;
        let event = CloudEvent::new(
            uuid::Uuid::new_v4().to_string(),
            "urn:delonix-seiva:data-layer",
            topic,
            data,
        );
        dorm_outbox::publish(topic, &event.to_json())
            .await
            .map(|_| ())
            .map_err(|e| Box::new(e) as BoxError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_ids_are_distinct() {
        let g = UuidIdGenerator;
        assert_ne!(g.new_id(), g.new_id());
    }

    #[test]
    fn system_clock_reads_a_time() {
        assert!(SystemClock.now() >= std::time::UNIX_EPOCH);
    }

    #[test]
    fn connect_builds_a_handle_and_enforces_the_invariants() {
        // deadpool is lazy, so a parseable DSN builds the handle without
        // connecting — enough to prove the gateway links against real DORM and
        // calls connect_with + strict sessions + the cursor key.
        let cfg = DormConfig {
            dsn: "host=localhost user=u dbname=x sslmode=disable".into(),
            replicas: vec![],
            read_only: false,
            schema: Some("notes".into()),
            pool_max: 4,
            cursor_key: b"test-secret".to_vec(),
        };
        let db = connect(&cfg).expect("the handle builds without connecting");
        assert_eq!(db.replica_count(), 0);
        assert!(
            dorm_core::db::strict_sessions(),
            "strict sessions must be on"
        );
    }
}
