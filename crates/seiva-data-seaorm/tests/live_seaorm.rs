//! Live proofs of the four `seiva-ports` the SeaORM adapter implements, against a
//! real PostgreSQL. Gated on `SEIVA_SEAORM_TEST_DSN` (a SeaORM Postgres URL, e.g.
//! `postgres://user@%2Fvar%2Frun%2Fpostgresql/seiva_seaorm_test`); every test skips
//! cleanly when it is unset, so the default `cargo test` stays offline.

#![cfg(feature = "seaorm")]

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use seiva_data_seaorm::{
    audit_migrations, connect, current_scope, idempotency_migrations, outbox_migrations,
    with_current_tx, SeaOrmAuditLog, SeaOrmAuditSink, SeaOrmConfig, SeaOrmIdempotencyStore,
    SeaOrmOutboxPublisher, SeaOrmUnitOfWork,
};
use seiva_ports::{
    AuditEntry, AuditLog, AuditQuery, AuditSink, BoxError, EventPublisher, IdempotencyOutcome,
    IdempotencyStore, Scope, UnitOfWork,
};
use std::sync::Arc;
use std::time::Duration;

/// The test connection, or `None` (skip) when `SEIVA_SEAORM_TEST_DSN` is unset.
async fn conn() -> Option<DatabaseConnection> {
    let dsn = std::env::var("SEIVA_SEAORM_TEST_DSN").ok()?;
    Some(
        connect(&SeaOrmConfig { dsn, pool_max: 16 })
            .await
            .expect("connect to the SeaORM test database"),
    )
}

/// Runs raw SQL on `conn`, panicking on error.
async fn run(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql).await.expect("statement runs");
}

/// Runs the framework DDL (all three tables) on `conn`.
async fn migrate(conn: &DatabaseConnection) {
    for stmt in outbox_migrations()
        .into_iter()
        .chain(idempotency_migrations())
        .chain(audit_migrations())
    {
        run(conn, &stmt).await;
    }
}

/// Reads a single `count(*)::int8` from a one-row query.
async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    conn.query_one_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .expect("query")
        .expect("one row")
        .try_get_by_index::<i64>(0)
        .expect("i64 column")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unit_of_work_commits_on_ok_and_rolls_back_on_err() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM unit-of-work proof");
        return;
    };
    migrate(&conn).await;
    let uow = SeaOrmUnitOfWork::new(conn.clone());

    // A committing transaction creates a table and inserts a row via the ambient tx.
    uow.transaction(Scope::Global, || {
        Box::pin(async {
            with_current_tx(|txn| {
                Box::pin(async move {
                    txn.execute_unprepared("DROP TABLE IF EXISTS seaorm_rt")
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                    txn.execute_unprepared(
                        "CREATE TABLE seaorm_rt (id int primary key, title text)",
                    )
                    .await
                    .map_err(|e| Box::new(e) as BoxError)?;
                    txn.execute_unprepared("INSERT INTO seaorm_rt (id, title) VALUES (1, 'hello')")
                        .await
                        .map(|_| ())
                        .map_err(|e| Box::new(e) as BoxError)
                })
            })
            .await
        })
    })
    .await
    .expect("the committing transaction succeeds");
    assert_eq!(
        count(&conn, "SELECT count(*)::int8 FROM seaorm_rt WHERE id = 1").await,
        1
    );

    // A failing transaction inserts row 2 then errors: the insert must roll back,
    // and the ORIGINAL typed boundary error must survive.
    #[derive(Debug)]
    struct Sentinel;
    impl std::fmt::Display for Sentinel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("deliberate failure")
        }
    }
    impl std::error::Error for Sentinel {}

    let err = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                with_current_tx(|txn| {
                    Box::pin(async move {
                        txn.execute_unprepared(
                            "INSERT INTO seaorm_rt (id, title) VALUES (2, 'doomed')",
                        )
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                        Err::<(), BoxError>(Box::new(Sentinel) as BoxError)
                    })
                })
                .await
            })
        })
        .await
        .expect_err("the failing transaction returns Err");
    assert!(
        err.downcast::<Sentinel>().is_ok(),
        "the typed boundary error survives the rollback"
    );
    assert_eq!(
        count(&conn, "SELECT count(*)::int8 FROM seaorm_rt").await,
        1,
        "the rolled-back row never persisted"
    );

    run(&conn, "DROP TABLE seaorm_rt").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbox_publisher_writes_in_the_same_transaction() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM outbox proof");
        return;
    };
    migrate(&conn).await;
    run(
        &conn,
        "DELETE FROM seiva_outbox WHERE topic = 'seaorm.test'",
    )
    .await;
    let uow = SeaOrmUnitOfWork::new(conn.clone());

    uow.transaction(Scope::Global, || {
        Box::pin(async {
            SeaOrmOutboxPublisher
                .publish("seaorm.test", br#"{"ok":true}"#)
                .await
        })
    })
    .await
    .expect("publish commits");

    let _ = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                SeaOrmOutboxPublisher
                    .publish("seaorm.test", b"rolled-back")
                    .await?;
                Err::<(), BoxError>("boom".into())
            })
        })
        .await;

    assert_eq!(
        count(
            &conn,
            "SELECT count(*)::int8 FROM seiva_outbox WHERE topic = 'seaorm.test'"
        )
        .await,
        1,
        "only the committed publish persisted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idempotency_one_effect_under_concurrency() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM idempotency proof");
        return;
    };
    migrate(&conn).await;
    const N: usize = 16;
    let uow = Arc::new(SeaOrmUnitOfWork::new(conn.clone()));

    run(&conn, "DROP TABLE IF EXISTS seaorm_idem_effect").await;
    run(
        &conn,
        "CREATE TABLE seaorm_idem_effect (id serial primary key)",
    )
    .await;
    run(
        &conn,
        "DELETE FROM seiva_idempotency WHERE key IN ('acme:k','bob:k')",
    )
    .await;

    async fn once(uow: &SeaOrmUnitOfWork, conn: DatabaseConnection, key: &str, fp: &str) -> bool {
        let key = key.to_owned();
        let fp = fp.to_owned();
        uow.transaction(Scope::Global, move || {
            Box::pin(async move {
                let store = SeaOrmIdempotencyStore::new(conn);
                if store.begin(&key, &fp, Duration::from_secs(300)).await?
                    == IdempotencyOutcome::Fresh
                {
                    with_current_tx(|txn| {
                        Box::pin(async move {
                            txn.execute_unprepared("INSERT INTO seaorm_idem_effect DEFAULT VALUES")
                                .await
                                .map(|_| ())
                                .map_err(|e| Box::new(e) as BoxError)
                        })
                    })
                    .await?;
                    store.finish(&key, b"effect-result").await?;
                    Ok::<bool, BoxError>(true)
                } else {
                    Ok(false)
                }
            })
        })
        .await
        .expect("request succeeds")
    }

    let mut handles = Vec::new();
    for _ in 0..N {
        let uow = Arc::clone(&uow);
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            once(&uow, conn, "acme:k", "fp-A").await
        }));
    }
    let mut fresh = 0;
    for h in handles {
        if h.await.expect("join") {
            fresh += 1;
        }
    }
    assert_eq!(
        fresh, 1,
        "exactly one connection may be Fresh under the race"
    );
    assert_eq!(
        count(&conn, "SELECT count(*)::int8 FROM seaorm_idem_effect").await,
        1,
        "the effect ran exactly once"
    );

    // Mismatch, Replay, independent key.
    let begin = |key: &'static str, fp: &'static str| {
        let uow = Arc::clone(&uow);
        let conn = conn.clone();
        async move {
            uow.transaction(Scope::Global, move || {
                Box::pin(async move {
                    SeaOrmIdempotencyStore::new(conn)
                        .begin(key, fp, Duration::from_secs(300))
                        .await
                })
            })
            .await
            .expect("begin")
        }
    };
    assert_eq!(
        begin("acme:k", "fp-DIFFERENT").await,
        IdempotencyOutcome::Mismatch
    );
    assert_eq!(
        begin("acme:k", "fp-A").await,
        IdempotencyOutcome::Replay(b"effect-result".to_vec())
    );
    assert_eq!(begin("bob:k", "fp-A").await, IdempotencyOutcome::Fresh);

    run(&conn, "DROP TABLE seaorm_idem_effect").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_sink_persists_a_redacted_entry() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM audit proof");
        return;
    };
    migrate(&conn).await;
    run(&conn, "DELETE FROM seiva_audit WHERE actor = 'alice'").await;

    let sink = SeaOrmAuditSink::new(conn.clone());
    let entry = AuditEntry::new("alice", "POST /v1/accounts/create_user", "201")
        .in_tenant(Some("acme"))
        .on_object(Some("user-1"))
        .with_request_id(Some("req-xyz"))
        .with_detail("username", "alice")
        .with_detail("password", "secret")
        .redacted();
    sink.record(&entry).await.expect("audit record persists");

    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT operation, outcome, details::text FROM seiva_audit WHERE actor = $1",
            vec![Value::String(Some("alice".to_owned()))],
        ))
        .await
        .expect("query")
        .expect("one row");
    assert_eq!(
        row.try_get_by_index::<String>(0).unwrap(),
        "POST /v1/accounts/create_user"
    );
    assert_eq!(row.try_get_by_index::<String>(1).unwrap(), "201");
    let details: String = row.try_get_by_index(2).unwrap();
    assert!(details.contains("[redacted]"), "the password is redacted");
    assert!(
        !details.contains("secret"),
        "the secret never persisted: {details}"
    );

    run(&conn, "DELETE FROM seiva_audit WHERE actor = 'alice'").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scope_is_surfaced_to_the_body() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM scope proof");
        return;
    };
    let uow = SeaOrmUnitOfWork::new(conn);
    assert!(current_scope().is_none());
    uow.transaction(Scope::tenant("acme").unwrap(), || {
        Box::pin(async {
            assert_eq!(current_scope(), Some(Scope::tenant("acme").unwrap()));
            Ok::<(), BoxError>(())
        })
    })
    .await
    .expect("tx");
}

/// Live proof of the **read** side ([`SeaOrmAuditLog`], ADR 0013): entries written
/// through [`SeaOrmAuditSink`] are read back by [`AuditLog::query`] **newest
/// first**, each filter (`actor`/`tenant`/`object`/`outcome`) narrows the page, and
/// the `effective_limit` bounds it — the SeaORM twin of the DORM gateway's
/// read-side proof. Gated on `SEIVA_SEAORM_TEST_DSN`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_log_reads_back_filtered_newest_first() {
    let Some(conn) = conn().await else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the SeaORM audit read-side proof");
        return;
    };
    migrate(&conn).await;
    run(
        &conn,
        "DELETE FROM seiva_audit WHERE actor IN ('reader-alice', 'reader-bob')",
    )
    .await;

    let sink = SeaOrmAuditSink::new(conn.clone());
    let log = SeaOrmAuditLog::new(conn.clone());

    // Three entries, recorded e1 → e2 → e3 (so e3 is newest). Two actors, two
    // tenants, two outcomes, so every filter has something to cut.
    let e1 = AuditEntry::new("reader-alice", "POST /v1/accounts/create_user", "201")
        .in_tenant(Some("acme"))
        .on_object(Some("user-1"));
    let e2 = AuditEntry::new("reader-bob", "DELETE /v1/accounts/user-2", "403")
        .in_tenant(Some("globex"))
        .on_object(Some("user-2"));
    let e3 = AuditEntry::new("reader-alice", "PATCH /v1/accounts/user-1", "200")
        .in_tenant(Some("acme"))
        .on_object(Some("user-1"));
    sink.record(&e1).await.expect("record e1");
    sink.record(&e2).await.expect("record e2");
    sink.record(&e3).await.expect("record e3");

    let alice = |limit: u32| AuditQuery {
        actor: Some("reader-alice".to_owned()),
        limit,
        ..AuditQuery::default()
    };

    // NEWEST FIRST: reader-alice's two rows come back e3 then e1.
    let page = log.query(&alice(0)).await.expect("query alice");
    assert_eq!(page.len(), 2, "two rows for reader-alice");
    assert_eq!(
        page[0].operation, "PATCH /v1/accounts/user-1",
        "newest first"
    );
    assert_eq!(page[1].operation, "POST /v1/accounts/create_user");
    assert_eq!(page[0].tenant.as_deref(), Some("acme"));
    assert_eq!(page[0].object.as_deref(), Some("user-1"));

    // LIMIT BOUND: effective_limit caps the page to the newest row only.
    let bounded = log.query(&alice(1)).await.expect("query bounded");
    assert_eq!(bounded.len(), 1, "limit 1 returns exactly one row");
    assert_eq!(bounded[0].operation, "PATCH /v1/accounts/user-1");

    // TENANT FILTER: globex has only reader-bob's denied delete.
    let globex = log
        .query(&AuditQuery {
            tenant: Some("globex".to_owned()),
            ..AuditQuery::default()
        })
        .await
        .expect("query globex");
    assert!(
        globex.iter().all(|e| e.tenant.as_deref() == Some("globex")),
        "the tenant filter admits only globex rows"
    );
    assert!(
        globex
            .iter()
            .any(|e| e.actor == "reader-bob" && e.outcome == "403"),
        "globex sees reader-bob's denied delete"
    );

    // OUTCOME FILTER (AND actor): only the 403 (reader-bob) among this test's rows.
    let denied = log
        .query(&AuditQuery {
            outcome: Some("403".to_owned()),
            actor: Some("reader-bob".to_owned()),
            ..AuditQuery::default()
        })
        .await
        .expect("query denied");
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].actor, "reader-bob");

    // OBJECT FILTER (AND actor): reader-alice on user-1 → both rows.
    let on_user1 = log
        .query(&AuditQuery {
            actor: Some("reader-alice".to_owned()),
            object: Some("user-1".to_owned()),
            ..AuditQuery::default()
        })
        .await
        .expect("query object");
    assert_eq!(on_user1.len(), 2, "both reader-alice rows target user-1");

    run(
        &conn,
        "DELETE FROM seiva_audit WHERE actor IN ('reader-alice', 'reader-bob')",
    )
    .await;
}
