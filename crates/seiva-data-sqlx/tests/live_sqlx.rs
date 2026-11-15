//! Live proofs of the four `seiva-ports` the sqlx adapter implements, against a
//! real PostgreSQL. Gated on `SEIVA_SQLX_TEST_DSN` (a sqlx Postgres URL, e.g.
//! `postgres:///seiva_sqlx_test?host=/var/run/postgresql`); every test skips
//! cleanly when it is unset, so the default `cargo test` stays offline.

#![cfg(feature = "sqlx")]

use seiva_data_sqlx::{
    audit_migrations, connect, current_scope, idempotency_migrations, outbox_migrations,
    with_current_tx, SqlxAuditSink, SqlxConfig, SqlxIdempotencyStore, SqlxOutboxPublisher,
    SqlxUnitOfWork,
};
use seiva_ports::{
    AuditEntry, AuditSink, BoxError, EventPublisher, IdempotencyOutcome, IdempotencyStore, Scope,
    UnitOfWork,
};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;

/// The test pool, or `None` (skip) when `SEIVA_SQLX_TEST_DSN` is unset.
async fn pool() -> Option<sqlx::PgPool> {
    let dsn = std::env::var("SEIVA_SQLX_TEST_DSN").ok()?;
    Some(
        connect(&SqlxConfig { dsn, pool_max: 16 })
            .await
            .expect("connect to the sqlx test database"),
    )
}

/// Runs the framework DDL (all three tables) on `pool`.
async fn migrate(pool: &sqlx::PgPool) {
    for stmt in outbox_migrations()
        .into_iter()
        .chain(idempotency_migrations())
        .chain(audit_migrations())
    {
        // The DDL is framework-owned (literal), not user input; `AssertSqlSafe`
        // is sqlx 0.9's required opt-in for a non-`'static` query string.
        sqlx::query(sqlx::AssertSqlSafe(stmt))
            .execute(pool)
            .await
            .expect("DDL applies");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unit_of_work_commits_on_ok_and_rolls_back_on_err() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the sqlx unit-of-work proof");
        return;
    };
    migrate(&pool).await;
    let uow = SqlxUnitOfWork::new(pool.clone());

    // A committing transaction creates a fresh table and inserts a row. The repo
    // reaches the ambient transaction through `with_current_tx`.
    uow.transaction(Scope::Global, || {
        Box::pin(async {
            with_current_tx(|tx| {
                Box::pin(async move {
                    sqlx::query("DROP TABLE IF EXISTS sqlx_rt")
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                    sqlx::query("CREATE TABLE sqlx_rt (id int primary key, title text)")
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| Box::new(e) as BoxError)?;
                    sqlx::query("INSERT INTO sqlx_rt (id, title) VALUES (1, 'hello')")
                        .execute(&mut **tx)
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

    // The commit is durable: a later transaction sees row 1.
    let seen: i64 = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                with_current_tx(|tx| {
                    Box::pin(async move {
                        let row = sqlx::query("SELECT count(*) AS n FROM sqlx_rt WHERE id = 1")
                            .fetch_one(&mut **tx)
                            .await
                            .map_err(|e| Box::new(e) as BoxError)?;
                        Ok(row.get::<i64, _>("n"))
                    })
                })
                .await
            })
        })
        .await
        .expect("read-back transaction succeeds");
    assert_eq!(seen, 1, "the committed row is visible afterwards");

    // A transaction that inserts row 2 then fails must NOT persist the insert, and
    // must surface the ORIGINAL typed boundary error (not a DB/stringified one).
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
                with_current_tx(|tx| {
                    Box::pin(async move {
                        sqlx::query("INSERT INTO sqlx_rt (id, title) VALUES (2, 'doomed')")
                            .execute(&mut **tx)
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

    // Row 2 is gone (rolled back); row 1 (committed) untouched.
    let count: i64 = sqlx::query("SELECT count(*) AS n FROM sqlx_rt")
        .fetch_one(&pool)
        .await
        .expect("count")
        .get("n");
    assert_eq!(count, 1, "the rolled-back row never persisted");

    sqlx::query("DROP TABLE sqlx_rt")
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbox_publisher_writes_in_the_same_transaction() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the sqlx outbox proof");
        return;
    };
    migrate(&pool).await;
    sqlx::query("DELETE FROM seiva_outbox WHERE topic = 'sqlx.test'")
        .execute(&pool)
        .await
        .expect("clean");
    let uow = SqlxUnitOfWork::new(pool.clone());

    // Publish inside a committing transaction → the row lands.
    uow.transaction(Scope::Global, || {
        Box::pin(async {
            SqlxOutboxPublisher
                .publish("sqlx.test", br#"{"ok":true}"#)
                .await
        })
    })
    .await
    .expect("publish commits");

    // Publish inside a FAILING transaction → the row rolls back with it.
    let _ = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                SqlxOutboxPublisher
                    .publish("sqlx.test", b"rolled-back")
                    .await?;
                Err::<(), BoxError>("boom".into())
            })
        })
        .await;

    let n: i64 = sqlx::query("SELECT count(*) AS n FROM seiva_outbox WHERE topic = 'sqlx.test'")
        .fetch_one(&pool)
        .await
        .expect("count")
        .get("n");
    assert_eq!(n, 1, "only the committed publish persisted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idempotency_one_effect_under_concurrency() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the sqlx idempotency proof");
        return;
    };
    migrate(&pool).await;
    const N: usize = 16;
    let uow = Arc::new(SqlxUnitOfWork::new(pool.clone()));

    // A clean key + a side-effect counter table.
    sqlx::query("DROP TABLE IF EXISTS sqlx_idem_effect")
        .execute(&pool)
        .await
        .expect("drop");
    sqlx::query("CREATE TABLE sqlx_idem_effect (id serial primary key)")
        .execute(&pool)
        .await
        .expect("create");
    sqlx::query("DELETE FROM seiva_idempotency WHERE key IN ('acme:k','bob:k')")
        .execute(&pool)
        .await
        .expect("clean keys");

    // One request: claim the key in the SAME transaction as the effect.
    async fn once(uow: &SqlxUnitOfWork, pool: sqlx::PgPool, key: &str, fp: &str) -> bool {
        let key = key.to_owned();
        let fp = fp.to_owned();
        uow.transaction(Scope::Global, move || {
            Box::pin(async move {
                let store = SqlxIdempotencyStore::new(pool);
                if store.begin(&key, &fp, Duration::from_secs(300)).await?
                    == IdempotencyOutcome::Fresh
                {
                    with_current_tx(|tx| {
                        Box::pin(async move {
                            sqlx::query("INSERT INTO sqlx_idem_effect DEFAULT VALUES")
                                .execute(&mut **tx)
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
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            once(&uow, pool, "acme:k", "fp-A").await
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

    let effects: i64 = sqlx::query("SELECT count(*) AS n FROM sqlx_idem_effect")
        .fetch_one(&pool)
        .await
        .expect("count")
        .get("n");
    assert_eq!(effects, 1, "the effect ran exactly once");

    // Mismatch (different fingerprint), Replay (stored bytes), independent key.
    let mismatch = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                SqlxIdempotencyStore::new(pool.clone())
                    .begin("acme:k", "fp-DIFFERENT", Duration::from_secs(300))
                    .await
            })
        })
        .await
        .expect("begin");
    assert_eq!(mismatch, IdempotencyOutcome::Mismatch);

    let replay = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                SqlxIdempotencyStore::new(pool.clone())
                    .begin("acme:k", "fp-A", Duration::from_secs(300))
                    .await
            })
        })
        .await
        .expect("begin");
    assert_eq!(
        replay,
        IdempotencyOutcome::Replay(b"effect-result".to_vec())
    );

    let other = uow
        .transaction(Scope::Global, || {
            Box::pin(async {
                SqlxIdempotencyStore::new(pool.clone())
                    .begin("bob:k", "fp-A", Duration::from_secs(300))
                    .await
            })
        })
        .await
        .expect("begin");
    assert_eq!(other, IdempotencyOutcome::Fresh);

    sqlx::query("DROP TABLE sqlx_idem_effect")
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_sink_persists_a_redacted_entry() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the sqlx audit proof");
        return;
    };
    migrate(&pool).await;
    sqlx::query("DELETE FROM seiva_audit WHERE actor = 'alice'")
        .execute(&pool)
        .await
        .expect("clean");

    let sink = SqlxAuditSink::new(pool.clone());
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

    let row = sqlx::query(
        "SELECT operation, outcome, details::text AS details \
         FROM seiva_audit WHERE actor = 'alice'",
    )
    .fetch_one(&pool)
    .await
    .expect("read-back");
    assert_eq!(
        row.get::<String, _>("operation"),
        "POST /v1/accounts/create_user"
    );
    assert_eq!(row.get::<String, _>("outcome"), "201");
    let details: String = row.get("details");
    assert!(details.contains("[redacted]"), "the password is redacted");
    assert!(
        !details.contains("secret"),
        "the secret never persisted: {details}"
    );

    sqlx::query("DELETE FROM seiva_audit WHERE actor = 'alice'")
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scope_is_surfaced_to_the_body() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the sqlx scope proof");
        return;
    };
    let uow = SqlxUnitOfWork::new(pool);
    // Outside a transaction there is no ambient scope.
    assert!(current_scope().is_none());
    // Inside, the body reads back exactly the scope it was opened for.
    let tenant = Scope::tenant("acme").expect("non-empty");
    uow.transaction(tenant.clone(), || {
        Box::pin(async {
            assert_eq!(current_scope(), Some(Scope::tenant("acme").unwrap()));
            Ok::<(), BoxError>(())
        })
    })
    .await
    .expect("tx");
    let _ = tenant;
}
