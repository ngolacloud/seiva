//! The DORM data-layer gateway (Parte C; Seiva ADR 0004).
//!
//! All coupling to DORM lives here. The rest of Seiva depends only on
//! `seiva-ports`; a module's `infrastructure` crate uses this gateway to build
//! its DORM handle and to satisfy the ports its `application` declared. Turning
//! the DORM data layer off is simply not compiling this crate (the brief's Parte
//! C), which is why the coupling is confined to one place.
//!
//! It depends on DORM by published crate version (resolved from crates.io) and
//! implements **all
//! four ports**: [`Clock`], [`IdGenerator`], the scope-based [`UnitOfWork`] via
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
use seiva_ports::{
    AuditEntry, AuditSink, BoxError, Clock, EventPublisher, Id, IdGenerator, IdempotencyOutcome,
    IdempotencyStore, PortResult, UnitOfWork,
};
use std::future::Future;
use std::pin::Pin;
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
    fn transaction<'a, T, F, Fut>(
        &'a self,
        work: F,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = PortResult<T>> + Send + 'a,
        T: Send + 'a,
    {
        let db = self.db.clone();
        Box::pin(async move {
            let outcome: Result<T, dorm_core::Error> =
                dorm_core::db::with_session(db, async move {
                    dorm_core::db::atomic(move |_tx| async move {
                        // Map the boundary error to a DORM error so `atomic` rolls
                        // back; the message is preserved across the bridge.
                        work()
                            .await
                            .map_err(|e| dorm_core::Error::Config(e.to_string()))
                    })
                    .await
                })
                .await;
            outcome.map_err(|e| Box::new(e) as BoxError)
        })
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

/// The DDL for the DORM idempotency table (`dorm_idempotency`), re-exported so a
/// composition root (or a migration) can create it. `CREATE TABLE IF NOT EXISTS`,
/// so running it twice is a no-op.
#[must_use]
pub fn idempotency_migrations() -> Vec<String> {
    dorm_core::idempotency::migrations()
}

/// The DORM-backed [`IdempotencyStore`] (ADR 0011): the durable, cross-replica
/// guarantee delegating to `dorm_core::idempotency`, whose claim is an atomic
/// `INSERT … ON CONFLICT DO NOTHING RETURNING` — a database CAS, not an in-memory
/// lock, so two replicas racing one key resolve to exactly one
/// [`Fresh`](IdempotencyOutcome::Fresh).
///
/// Call `begin`/`finish` **inside** a [`DormUnitOfWork::transaction`]: the
/// reservation then commits in the same transaction as the domain mutation and
/// the outbox write (`DormOutboxPublisher`), so a committed key always has its
/// effect, and a rollback un-claims the key. A fingerprint mismatch is reported
/// as [`IdempotencyOutcome::Mismatch`] (not an error), so the caller answers
/// `409` without executing.
///
/// The response is opaque bytes to the port; this adapter stores them losslessly
/// in the JSONB `response` column as a byte array and reads them back on replay.
pub struct DormIdempotencyStore;

/// Encodes opaque response bytes as a JSON value for the JSONB column (a byte
/// array — lossless for any bytes, no base64 dependency).
fn bytes_to_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::Value::Array(bytes.iter().map(|b| serde_json::json!(*b)).collect())
}

/// Reads the response bytes back from the stored JSON value.
fn json_to_bytes(value: &serde_json::Value) -> PortResult<Vec<u8>> {
    let array = value.as_array().ok_or_else(|| {
        Box::<dyn std::error::Error + Send + Sync>::from("stored response is not a byte array")
    })?;
    array
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|n| *n <= u64::from(u8::MAX))
                .map(|n| n as u8)
                .ok_or_else(|| {
                    Box::<dyn std::error::Error + Send + Sync>::from(
                        "stored response byte out of range",
                    ) as BoxError
                })
        })
        .collect()
}

#[async_trait]
impl IdempotencyStore for DormIdempotencyStore {
    async fn begin(
        &self,
        scoped_key: &str,
        fingerprint: &str,
        ttl: std::time::Duration,
    ) -> PortResult<IdempotencyOutcome> {
        let ttl_secs = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
        match dorm_core::idempotency::begin(scoped_key, fingerprint, ttl_secs).await {
            Ok(dorm_core::idempotency::Begin::Fresh) => Ok(IdempotencyOutcome::Fresh),
            Ok(dorm_core::idempotency::Begin::InProgress) => Ok(IdempotencyOutcome::InProgress),
            Ok(dorm_core::idempotency::Begin::Replay(value)) => {
                Ok(IdempotencyOutcome::Replay(json_to_bytes(&value)?))
            }
            // A reused key with a different fingerprint is an OUTCOME, not a
            // failure: the caller returns 409 without executing.
            Err(dorm_core::Error::IdempotencyMismatch { .. }) => Ok(IdempotencyOutcome::Mismatch),
            Err(e) => Err(Box::new(e) as BoxError),
        }
    }

    async fn finish(&self, scoped_key: &str, response: &[u8]) -> PortResult<()> {
        dorm_core::idempotency::finish(scoped_key, &bytes_to_json(response))
            .await
            .map_err(|e| Box::new(e) as BoxError)
    }

    async fn purge_expired(&self) -> PortResult<u64> {
        dorm_core::idempotency::purge_expired()
            .await
            .map_err(|e| Box::new(e) as BoxError)
    }
}

/// The DDL for the audit table (`seiva_audit`, ADR 0013), re-exported so a
/// composition root (or a migration) can create it. `CREATE TABLE IF NOT EXISTS`.
#[must_use]
pub fn audit_migrations() -> Vec<String> {
    vec![format!(
        "CREATE TABLE IF NOT EXISTS {TABLE} (\
           id bigserial PRIMARY KEY, \
           at TIMESTAMPTZ NOT NULL DEFAULT now(), \
           actor TEXT NOT NULL, \
           tenant TEXT, \
           operation TEXT NOT NULL, \
           object TEXT, \
           outcome TEXT NOT NULL, \
           request_id TEXT, \
           details JSONB NOT NULL DEFAULT '{{}}')",
        TABLE = "seiva_audit"
    )]
}

/// The DORM-backed [`AuditSink`] (ADR 0013): appends each entry to `seiva_audit`.
///
/// Unlike the idempotency store, audit does **not** join the request's
/// transaction — it opens its own session per record (so the audit middleware,
/// which runs above any transaction, can use it) and records the outcome of both
/// successful and denied mutations. Entries arrive already redacted.
pub struct DormAuditSink {
    db: Arc<dorm_core::db::Database>,
}

impl DormAuditSink {
    /// Wraps a [`connect`]-built handle as an audit sink.
    #[must_use]
    pub fn new(db: dorm_core::db::Database) -> Self {
        Self { db: Arc::new(db) }
    }
}

#[async_trait]
impl AuditSink for DormAuditSink {
    async fn record(&self, entry: &AuditEntry) -> PortResult<()> {
        use dorm_core::value::Value;
        let opt = |v: &Option<String>| v.clone().map_or(Value::Null, Value::Text);
        let details = serde_json::Value::Object(
            entry
                .details
                .iter()
                .cloned()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect(),
        );
        let params = vec![
            Value::Text(entry.actor.clone()),
            opt(&entry.tenant),
            Value::Text(entry.operation.clone()),
            opt(&entry.object),
            Value::Text(entry.outcome.clone()),
            opt(&entry.request_id),
            Value::Json(details),
        ];
        let db = self.db.clone();
        dorm_core::db::with_session(db, async move {
            dorm_core::db::run(
                "INSERT INTO seiva_audit \
                 (actor, tenant, operation, object, outcome, request_id, details) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &params,
            )
            .await
        })
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

    /// Live round-trip against a real PostgreSQL, proving the gateway's
    /// [`UnitOfWork`] actually commits on `Ok` and rolls back on `Err` through
    /// DORM (`with_session` + `db::atomic`). Gated on `SEIVA_DORM_TEST_DSN`
    /// (a libpq DSN, e.g. `host=/var/run/postgresql dbname=seiva_dorm_test`);
    /// skips cleanly when it is unset so the default `cargo test` stays offline.
    #[tokio::test]
    async fn live_unit_of_work_commits_and_rolls_back() {
        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live round-trip");
            return;
        };
        let uow = DormUnitOfWork::new(
            connect(&DormConfig {
                dsn,
                replicas: vec![],
                read_only: false,
                schema: None,
                pool_max: 4,
                cursor_key: b"live-test-secret".to_vec(),
            })
            .expect("connect to the test database"),
        );
        let exec = |sql: &'static str| async move {
            dorm_core::db::execute(sql, &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)
        };

        // A clean slate, then a committing transaction creates and inserts.
        uow.transaction(|| async {
            exec("DROP TABLE IF EXISTS seiva_rt").await?;
            exec("CREATE TABLE seiva_rt (id int primary key, title text)").await?;
            exec("INSERT INTO seiva_rt (id, title) VALUES (1, 'hello')").await?;
            Ok(())
        })
        .await
        .expect("the committing transaction succeeds");

        // Commit is durable: a later transaction sees row 1 (UPDATE affects 1).
        let updated = uow
            .transaction(|| async {
                exec("UPDATE seiva_rt SET title = 'again' WHERE id = 1").await
            })
            .await
            .expect("read-back transaction succeeds");
        assert_eq!(updated, 1, "the committed row must be visible afterwards");

        // Rollback: insert row 2 then fail — the insert must NOT persist.
        let rolled_back = uow
            .transaction(|| async {
                exec("INSERT INTO seiva_rt (id, title) VALUES (2, 'doomed')").await?;
                Err::<(), BoxError>("deliberate failure to force rollback".into())
            })
            .await;
        assert!(rolled_back.is_err(), "the failing transaction returns Err");

        // Row 2 is gone (DELETE affects 0 rows); row 1 (committed) is untouched.
        let deleted = uow
            .transaction(|| async { exec("DELETE FROM seiva_rt WHERE id = 2").await })
            .await
            .expect("verification transaction succeeds");
        assert_eq!(deleted, 0, "the rolled-back row must not have persisted");

        uow.transaction(|| async { exec("DROP TABLE seiva_rt").await })
            .await
            .expect("cleanup succeeds");
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

    #[test]
    fn response_bytes_round_trip_through_the_json_column() {
        for case in [&b""[..], b"{\"ok\":true}", &[0u8, 1, 2, 255, 128]] {
            let json = bytes_to_json(case);
            assert_eq!(json_to_bytes(&json).unwrap(), case.to_vec());
        }
    }

    /// The distributed, durable idempotency proof against a real PostgreSQL
    /// (ADR 0011). Gated on `SEIVA_DORM_TEST_DSN`; skips cleanly when unset.
    ///
    /// It drives many **concurrent connections** (the replica model) at one key,
    /// each running `begin` → effect → `finish` in a single transaction via the
    /// gateway's own [`DormUnitOfWork`] + [`DormIdempotencyStore`]. The claim is a
    /// database CAS (`INSERT … ON CONFLICT`), so exactly one connection is `Fresh`
    /// and performs the effect; the rest block on the key row, then `Replay`. We
    /// assert: one effect despite the race; a different fingerprint is a
    /// `Mismatch` (no effect); a completed key replays the stored bytes; a
    /// different key is independent.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn live_idempotency_one_effect_under_concurrency() {
        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live idempotency proof");
            return;
        };
        const N: usize = 20;

        let uow = Arc::new(DormUnitOfWork::new(
            connect(&DormConfig {
                dsn,
                replicas: vec![],
                read_only: false,
                schema: None,
                pool_max: N + 4,
                cursor_key: b"live-idem-secret".to_vec(),
            })
            .expect("connect to the test database"),
        ));

        // Schema: the idempotency table (via the shipped DDL) + a side-effect
        // table whose row count is the number of times the effect ran.
        let ddl = idempotency_migrations();
        uow.transaction(|| async {
            for stmt in &ddl {
                dorm_core::db::execute(stmt, &[])
                    .await
                    .map_err(|e| Box::new(e) as BoxError)?;
            }
            dorm_core::db::execute("DROP TABLE IF EXISTS seiva_idem_effect", &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
            dorm_core::db::execute(
                "CREATE TABLE seiva_idem_effect (id serial primary key, note text)",
                &[],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            // A clean key for this run.
            dorm_core::db::execute(
                "DELETE FROM dorm_idempotency WHERE key IN ('acme:k','acme:k2','bob:k')",
                &[],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            Ok(())
        })
        .await
        .expect("setup succeeds");

        // One request: claim the key in the SAME transaction as the effect, so a
        // duplicate blocks on the key row and replays after commit — exactly one
        // effect. Returns whether THIS connection performed the effect.
        async fn once(uow: &DormUnitOfWork, key: &str, fingerprint: &str) -> PortResult<bool> {
            let key = key.to_owned();
            let fingerprint = fingerprint.to_owned();
            uow.transaction(move || async move {
                let store = DormIdempotencyStore;
                match store
                    .begin(&key, &fingerprint, std::time::Duration::from_secs(300))
                    .await?
                {
                    IdempotencyOutcome::Fresh => {
                        dorm_core::db::execute(
                            "INSERT INTO seiva_idem_effect (note) VALUES ('effect')",
                            &[],
                        )
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                        store.finish(&key, b"effect-result").await?;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            })
            .await
        }

        // N concurrent connections race the same key with the same fingerprint.
        let mut handles = Vec::new();
        for _ in 0..N {
            let uow = Arc::clone(&uow);
            handles.push(tokio::spawn(
                async move { once(&uow, "acme:k", "fp-A").await },
            ));
        }
        let mut fresh = 0usize;
        for h in handles {
            if h.await.expect("task joins").expect("request succeeds") {
                fresh += 1;
            }
        }
        assert_eq!(
            fresh, 1,
            "exactly one connection may be Fresh under the race"
        );

        // The side effect ran exactly once despite N concurrent requests.
        let count = uow
            .transaction(|| async {
                dorm_core::db::sql("SELECT count(*)::int FROM seiva_idem_effect", &[])
                    .await
                    .map_err(|e| Box::new(e) as BoxError)
            })
            .await
            .expect("count succeeds");
        let effects: i32 = count[0].try_get(0).expect("count column");
        assert_eq!(effects, 1, "the effect must have run exactly once");

        // A reused key with a DIFFERENT fingerprint is a Mismatch (no effect).
        let mismatch = uow
            .transaction(|| async {
                DormIdempotencyStore
                    .begin(
                        "acme:k",
                        "fp-DIFFERENT",
                        std::time::Duration::from_secs(300),
                    )
                    .await
            })
            .await
            .expect("begin succeeds");
        assert_eq!(mismatch, IdempotencyOutcome::Mismatch);

        // The completed key replays the exact stored bytes, no re-execution.
        let replay = uow
            .transaction(|| async {
                DormIdempotencyStore
                    .begin("acme:k", "fp-A", std::time::Duration::from_secs(300))
                    .await
            })
            .await
            .expect("begin succeeds");
        assert_eq!(
            replay,
            IdempotencyOutcome::Replay(b"effect-result".to_vec())
        );

        // A different key (another tenant/operation) is wholly independent.
        let other = uow
            .transaction(|| async {
                DormIdempotencyStore
                    .begin("bob:k", "fp-A", std::time::Duration::from_secs(300))
                    .await
            })
            .await
            .expect("begin succeeds");
        assert_eq!(other, IdempotencyOutcome::Fresh);

        // Cleanup.
        uow.transaction(|| async {
            dorm_core::db::execute("DROP TABLE seiva_idem_effect", &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)
        })
        .await
        .expect("cleanup succeeds");
    }

    /// Live proof of consumer-side inbox dedup and TTL purge against real
    /// PostgreSQL (ADR 0011). Gated on `SEIVA_DORM_TEST_DSN`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_inbox_dedup_and_purge() {
        use seiva_ports::inbox_key;

        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live inbox/purge proof");
            return;
        };
        let uow = DormUnitOfWork::new(
            connect(&DormConfig {
                dsn,
                replicas: vec![],
                read_only: false,
                schema: None,
                pool_max: 4,
                cursor_key: b"live-inbox-secret".to_vec(),
            })
            .expect("connect to the test database"),
        );

        // Setup: the idempotency table + a consumed-effects counter table.
        uow.transaction(|| async {
            for stmt in &idempotency_migrations() {
                dorm_core::db::execute(stmt, &[])
                    .await
                    .map_err(|e| Box::new(e) as BoxError)?;
            }
            dorm_core::db::execute("DROP TABLE IF EXISTS seiva_inbox_effect", &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
            dorm_core::db::execute(
                "CREATE TABLE seiva_inbox_effect (id serial primary key)",
                &[],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            dorm_core::db::execute(
                "DELETE FROM dorm_idempotency WHERE key LIKE 'inbox:billing:%' OR key = 'ttl-key'",
                &[],
            )
            .await
            .map_err(|e| Box::new(e) as BoxError)?;
            Ok(())
        })
        .await
        .expect("setup succeeds");

        // Deliver the SAME event to the same consumer twice: the effect+mark run
        // in one transaction, so the redelivery is skipped — one effect.
        let key = inbox_key("billing", "evt-1");
        for _ in 0..2 {
            let key = key.clone();
            uow.transaction(move || async move {
                let store = DormIdempotencyStore;
                if store
                    .begin(&key, "evt-1", std::time::Duration::from_secs(300))
                    .await?
                    == IdempotencyOutcome::Fresh
                {
                    dorm_core::db::execute("INSERT INTO seiva_inbox_effect DEFAULT VALUES", &[])
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                    store.finish(&key, b"").await?;
                }
                Ok(())
            })
            .await
            .expect("consume succeeds");
        }
        let count = uow
            .transaction(|| async {
                dorm_core::db::sql("SELECT count(*)::int FROM seiva_inbox_effect", &[])
                    .await
                    .map_err(|e| Box::new(e) as BoxError)
            })
            .await
            .expect("count succeeds");
        let effects: i32 = count[0].try_get(0).expect("count column");
        assert_eq!(effects, 1, "a redelivered event must be consumed once");

        // TTL purge: a key claimed with ttl 0 is already expired; purge removes it.
        uow.transaction(|| async {
            DormIdempotencyStore
                .begin("ttl-key", "fp", std::time::Duration::from_secs(0))
                .await
                .map(|_| ())
        })
        .await
        .expect("claim succeeds");
        let purged = uow
            .transaction(|| async { DormIdempotencyStore.purge_expired().await })
            .await
            .expect("purge succeeds");
        assert!(
            purged >= 1,
            "the expired key must be purged (removed {purged})"
        );
        // The purged key is free to claim again (Fresh).
        let reclaim = uow
            .transaction(|| async {
                DormIdempotencyStore
                    .begin("ttl-key", "fp", std::time::Duration::from_secs(300))
                    .await
            })
            .await
            .expect("begin succeeds");
        assert_eq!(reclaim, IdempotencyOutcome::Fresh);

        // Cleanup.
        uow.transaction(|| async {
            dorm_core::db::execute("DROP TABLE seiva_inbox_effect", &[])
                .await
                .map_err(|e| Box::new(e) as BoxError)
        })
        .await
        .expect("cleanup succeeds");
    }

    /// Live proof that the audit sink persists a (redacted) entry to `seiva_audit`
    /// against real PostgreSQL (ADR 0013). Gated on `SEIVA_DORM_TEST_DSN`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_audit_sink_persists_a_redacted_entry() {
        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live audit proof");
            return;
        };
        let cfg = DormConfig {
            dsn,
            replicas: vec![],
            read_only: false,
            schema: None,
            pool_max: 4,
            cursor_key: b"live-audit-secret".to_vec(),
        };
        // One handle for the sink, a second for the test's own setup/read-back.
        let sink = DormAuditSink::new(connect(&cfg).expect("connect the sink"));
        let probe = Arc::new(connect(&cfg).expect("connect the probe"));

        // Create the table and clean a prior run, via a one-off session.
        let ddl = audit_migrations();
        dorm_core::db::with_session(probe.clone(), async move {
            for stmt in &ddl {
                dorm_core::db::run_ddl(stmt).await?;
            }
            dorm_core::db::run("DELETE FROM seiva_audit WHERE actor = 'alice'", &[]).await?;
            Ok::<_, dorm_core::Error>(())
        })
        .await
        .expect("audit setup succeeds");

        // Record an entry whose sensitive detail was redacted by the caller.
        let entry = AuditEntry {
            actor: "alice".to_owned(),
            tenant: Some("acme".to_owned()),
            operation: "POST /v1/accounts/create_user".to_owned(),
            object: Some("user-1".to_owned()),
            outcome: "201".to_owned(),
            request_id: Some("req-xyz".to_owned()),
            details: vec![
                ("username".to_owned(), "alice".to_owned()),
                ("password".to_owned(), "secret".to_owned()),
            ],
        }
        .redacted();
        sink.record(&entry).await.expect("audit record persists");

        // Read it back and assert the row + that the secret never landed.
        let rows = dorm_core::db::with_session(probe.clone(), async move {
            dorm_core::db::sql(
                "SELECT operation, outcome, tenant, object, request_id, details::text \
                 FROM seiva_audit WHERE actor = 'alice'",
                &[],
            )
            .await
        })
        .await
        .expect("read-back succeeds");
        assert_eq!(rows.len(), 1, "exactly one audit row");
        let operation: String = rows[0].try_get(0).expect("operation");
        let outcome: String = rows[0].try_get(1).expect("outcome");
        let details: String = rows[0].try_get(5).expect("details");
        assert_eq!(operation, "POST /v1/accounts/create_user");
        assert_eq!(outcome, "201");
        assert!(details.contains("\"username\""));
        assert!(
            details.contains("[redacted]"),
            "the password value is redacted"
        );
        assert!(
            !details.contains("secret"),
            "the secret value never persisted: {details}"
        );

        // Cleanup.
        dorm_core::db::with_session(probe, async move {
            dorm_core::db::run("DELETE FROM seiva_audit WHERE actor = 'alice'", &[]).await
        })
        .await
        .expect("cleanup succeeds");
    }
}
