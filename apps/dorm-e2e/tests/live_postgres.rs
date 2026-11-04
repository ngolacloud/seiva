//! Live-Postgres integration tests for the DORM-backed `accounts` persistence
//! path — the increment [`dorm_e2e`]'s library docs call out as "the live job".
//!
//! These tests exercise the **real** `DormCredentialRepository` and the real
//! `dorm-outbox` relay against a running PostgreSQL server, so they prove the
//! write → read-back and the outbox → relay → publisher paths end to end rather
//! than only at compile time (which the crate's offline tests already do).
//!
//! # Environment gating — why the offline `cargo test` stays green
//!
//! Every test here begins by reading [`DORM_TEST_DSN`](TEST_DSN_VAR) through
//! [`test_dsn`]. When that variable is **unset or empty** — the normal case for
//! the offline gates and a developer with no database — [`test_dsn`] prints a
//! skip line and returns `None`, and the test body returns at once as a green
//! skip. Nothing connects, so `cargo test` passes with no database present.
//!
//! When CI (or a developer) **sets** `DORM_TEST_DSN` to a reachable server, the
//! live body runs: it builds a [`Database`](dorm_core::db::Database) with
//! [`connect`], creates the tables it needs, drives the repository / relay, and
//! asserts the real behaviour. This is the same technique the crate's library
//! uses to stay buildable without DORM's private repo: the gate is data, not a
//! Cargo feature, so the single test binary covers both the offline and the live
//! worlds.
//!
//! # What the CI Postgres service must provide
//!
//! * A reachable server whose **database already exists** — [`connect`] builds a
//!   lazy pool but does not create databases, so the DSN must name a database
//!   that is present (for example `dorm_e2e`).
//! * No schema setup is required: the tests create every table they touch
//!   themselves (`accounts_users` via [`users_table_ddl`] and the four
//!   `dorm-outbox` tables via [`dorm_outbox::migrations`]), all with
//!   `CREATE TABLE IF NOT EXISTS`, and they run under the default `public`
//!   search path ([`DormConfig::schema`] is `None` here).
//!
//! `DORM_TEST_DSN` is a libpq/`tokio-postgres` connection string, in either the
//! keyword/value form
//! `host=localhost port=5432 user=postgres password=postgres dbname=dorm_e2e sslmode=disable`
//! or the URL form
//! `postgres://postgres:postgres@localhost:5432/dorm_e2e`.

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use accounts_application::CredentialRepository;
use accounts_domain::User;
use accounts_facade::UserDto;
use dorm_core::db::{self, with_session};
use dorm_core::value::Value;
use dorm_e2e::{
    run_relay, users_table_ddl, DormCredentialRepository, LoggingPublisher, RelaySettings,
    USERS_TABLE,
};
use seiva_auth::{hash_password, verify_password};
use seiva_data_dorm::{connect, DormConfig, UuidIdGenerator};
use seiva_messaging::{CloudEvent, Publisher};
use seiva_ports::IdGenerator;

/// The environment variable that gates the live tests. When unset or empty the
/// tests skip; when set to a reachable DSN they run against it.
const TEST_DSN_VAR: &str = "DORM_TEST_DSN";

/// Fixed, non-secret signing key the live tests hand [`connect`] for the keyset
/// cursor. A test never mints a real cursor, but `connect` refuses a forgeable
/// empty default, so a value must be supplied.
const TEST_CURSOR_KEY: &[u8] = b"dorm-e2e-live-postgres-test-cursor-key";

/// Reads [`TEST_DSN_VAR`] and decides whether the calling test runs live.
///
/// Returns `Some(dsn)` when the variable is set to a non-empty value (the live
/// path), or `None` after printing a skip line when it is unset or empty (the
/// green-skip path that keeps the offline `cargo test` passing with no
/// database). This single helper is the whole env gate: every test calls it
/// first and returns immediately on `None`.
fn test_dsn() -> Option<String> {
    match std::env::var(TEST_DSN_VAR) {
        Ok(dsn) if !dsn.trim().is_empty() => Some(dsn),
        _ => {
            eprintln!("skipping live-Postgres test: {TEST_DSN_VAR} not set");
            None
        }
    }
}

/// Builds a small, read-write [`DormConfig`] for `dsn`: no replicas, the default
/// `public` schema (`None`), a tiny pool, and the fixed test cursor key. This is
/// the module's handle the live body drives sessions over.
fn test_config(dsn: &str) -> DormConfig {
    DormConfig {
        dsn: dsn.to_string(),
        replicas: vec![],
        read_only: false,
        schema: None,
        pool_max: 4,
        cursor_key: TEST_CURSOR_KEY.to_vec(),
    }
}

/// A per-run-unique suffix (nanoseconds since the Unix epoch) so re-runs do not
/// collide on the `accounts_users.username` unique key or on an outbox topic.
///
/// A library crate must not read the wall clock on its own (it would be
/// non-deterministic), but a test legitimately may: the uniqueness it buys is
/// exactly what lets the same live database accept repeated runs.
fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A [`Publisher`] that forwards to a shared [`LoggingPublisher`] behind an
/// `Arc`, so the relay worker (which consumes its publisher by value) can record
/// into an instance the test still holds and can assert on afterwards.
struct SharedPublisher(Arc<LoggingPublisher>);

impl Publisher for SharedPublisher {
    fn publish(
        &self,
        topic: &str,
        event: &CloudEvent,
    ) -> impl Future<Output = seiva_messaging::Result<()>> {
        self.0.publish(topic, event)
    }
}

/// The required deliverable: a real round-trip through [`DormCredentialRepository`]
/// against a live PostgreSQL server.
///
/// The use case's own `create_user` is deliberately **not** used to drive this:
/// its DORM event publisher serializes the use case's placeholder payload (raw
/// id bytes) as JSON, which is not valid JSON, so the real path would fail at
/// the outbox publish. Driving the repository directly is the honest way to
/// exercise `save` + `find_by_username` end to end; the outbox path gets its own
/// test below.
///
/// Steps: create `accounts_users`; hash a known password with the same
/// `seiva-auth` the application layer uses; `save` the `User` inside a
/// transaction; read it back with `find_by_username`; assert the stored
/// credential is an Argon2id PHC string that verifies the right password and
/// rejects a wrong one, that the plaintext was never stored, and that the public
/// DTO carries no secret; then delete the row (ignoring cleanup errors).
#[tokio::test]
async fn accounts_round_trips_against_real_postgres() {
    let Some(dsn) = test_dsn() else {
        return;
    };

    let cfg = test_config(&dsn);
    let db = Arc::new(connect(&cfg).expect("the DORM handle builds from the test DSN"));

    // 1. Ensure the table exists (idempotent DDL, autocommitted in the session).
    with_session(db.clone(), async { db::run(&users_table_ddl(), &[]).await })
        .await
        .expect("the accounts_users table is created");

    // 2. Build a user with a real Argon2id hash, exactly as the application layer
    //    would: the domain and repository only ever see the PHC string.
    let password = "correct horse battery staple";
    let password_hash = hash_password(password).expect("argon2id hashing succeeds");
    let suffix = unique_suffix();
    let username = format!("live-user-{suffix}");
    let id = u128::from_be_bytes(UuidIdGenerator.new_id().0);
    let user =
        User::new(id, username.clone(), password_hash).expect("the user satisfies the rules");

    // 3. Persist through the real repository, inside a transaction (as the
    //    unit-of-work would), so the row is committed before the read-back.
    let repo = DormCredentialRepository;
    with_session(db.clone(), async {
        db::atomic(|_tx| async {
            repo.save(&user)
                .await
                .map_err(|e| dorm_core::Error::Config(e.to_string()))
        })
        .await
    })
    .await
    .expect("the user is saved and the transaction commits");

    // 4. Read it back through the real repository and assert the invariants.
    let found = with_session(db.clone(), async {
        DormCredentialRepository.find_by_username(&username).await
    })
    .await
    .expect("the read-back query runs")
    .expect("the saved user is found by username");

    assert_eq!(found.id, id, "the id round-trips through TEXT");
    assert_eq!(found.username, username);
    assert!(
        found.password_hash.starts_with("$argon2id$"),
        "the stored credential must be an argon2id PHC string, got: {}",
        found.password_hash
    );
    assert_ne!(
        found.password_hash, password,
        "the plaintext password must never be stored"
    );
    assert!(
        verify_password(password, &found.password_hash).expect("the PHC string parses"),
        "the stored hash verifies the correct password"
    );
    assert!(
        !verify_password("the-wrong-password", &found.password_hash)
            .expect("the PHC string parses"),
        "the stored hash rejects a wrong password"
    );

    // The value crossing the module boundary is the public DTO, which has only an
    // id and a username — there is structurally nowhere for a secret to sit.
    let dto = UserDto {
        id: found.id,
        username: found.username.clone(),
    };
    assert_eq!(dto.id, id);
    assert_eq!(dto.username, username);
    assert_ne!(dto.username, password, "the DTO carries no plaintext");

    // 5. Clean up the row we created. Failures here must not fail the test.
    let _ = with_session(db.clone(), async {
        db::run(
            &format!("DELETE FROM {USERS_TABLE} WHERE username = $1"),
            &[Value::Text(username.clone())],
        )
        .await
    })
    .await;
}

/// The optional deliverable: a published outbox event is forwarded by one live
/// relay pass to the `Publisher`.
///
/// The `dorm-outbox` schema is cheap to create ([`dorm_outbox::migrations`]
/// returns the `CREATE TABLE IF NOT EXISTS` statements), so this proves the full
/// `write → outbox → relay → publisher` path live. A per-run-unique topic and
/// consumer keep the cursor at zero so the freshly published event is always
/// seen, and avoid colliding with other runs on the shared tables.
#[tokio::test]
async fn outbox_relay_forwards_a_published_event_against_real_postgres() {
    let Some(dsn) = test_dsn() else {
        return;
    };

    let cfg = test_config(&dsn);
    let db = Arc::new(connect(&cfg).expect("the DORM handle builds from the test DSN"));

    let suffix = unique_suffix();
    let topic = format!("accounts.user_created.{suffix}");
    let consumer = format!("dorm-e2e-relay-{suffix}");

    // 1. Create the outbox/inbox/cursor/dead tables (idempotent).
    with_session(db.clone(), async {
        for ddl in dorm_outbox::migrations() {
            db::run(&ddl, &[]).await?;
        }
        Ok::<(), dorm_core::Error>(())
    })
    .await
    .expect("the dorm-outbox tables are created");

    // 2. Publish one event into the outbox, inside a transaction (the outbox
    //    refuses a publish outside one — that is what makes it transactional).
    let event = CloudEvent::new(
        format!("evt-{suffix}"),
        "/accounts",
        topic.clone(),
        serde_json::json!({ "username": format!("live-{suffix}") }),
    );
    let payload = event.to_json();
    with_session(db.clone(), async {
        db::atomic(|_tx| async { dorm_outbox::publish(&topic, &payload).await.map(|_| ()) }).await
    })
    .await
    .expect("the event is written to the outbox");

    // 3. Run exactly one relay pass (shutdown is already resolved), bound to the
    //    session so the relay's db:: calls run against our handle, and record
    //    what it forwards into a LoggingPublisher we still hold.
    let publisher = Arc::new(LoggingPublisher::new());
    let settings = RelaySettings {
        consumer,
        topic: topic.clone(),
        interval: Duration::ZERO,
        drain_threshold: 0,
        ..RelaySettings::default()
    };
    let report = with_session(
        db.clone(),
        run_relay(settings, async {}, SharedPublisher(publisher.clone())),
    )
    .await;

    assert!(report.passes >= 1, "at least one relay pass runs");
    assert_eq!(
        report.processed, 1,
        "the single published event is forwarded"
    );
    assert_eq!(report.errors, 0, "the live pass does not error");

    let sent = publisher.sent();
    assert_eq!(sent.len(), 1, "the publisher recorded exactly one event");
    assert_eq!(
        sent[0].id, event.id,
        "the forwarded event is the one published"
    );
    assert_eq!(sent[0].ty, topic);

    // 4. Clean up the rows we created for this unique topic. Ignore errors.
    let _ = with_session(db.clone(), async {
        db::run(
            &format!("DELETE FROM {} WHERE topic = $1", dorm_outbox::OUTBOX),
            &[Value::Text(topic.clone())],
        )
        .await?;
        db::run(
            &format!("DELETE FROM {} WHERE topic = $1", dorm_outbox::CURSOR),
            &[Value::Text(topic.clone())],
        )
        .await?;
        Ok::<(), dorm_core::Error>(())
    })
    .await;
}
