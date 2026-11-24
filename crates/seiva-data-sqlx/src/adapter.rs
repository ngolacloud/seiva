//! The sqlx adapter proper, gated behind the `sqlx` feature.

use async_trait::async_trait;
use seiva_ports::{
    AuditEntry, AuditLog, AuditQuery, AuditSink, BoxError, EventPublisher, IdempotencyOutcome,
    IdempotencyStore, PortResult, Scope, UnitOfWork,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Boxes any concrete error as the port boundary's opaque [`BoxError`].
fn boxed<E: std::error::Error + Send + Sync + 'static>(e: E) -> BoxError {
    Box::new(e)
}

/// The error a port returns when it is driven outside a [`UnitOfWork::transaction`]
/// — the ambient transaction the sqlx ports require is simply not there.
fn no_ambient_tx() -> BoxError {
    "no ambient sqlx transaction in scope: call this inside UnitOfWork::transaction".into()
}

/// The state bound into the task-local for the duration of one unit of work: the
/// live transaction (an `Option` so it can be taken back out to commit/roll back
/// once the body returns) and the [`Scope`] it was opened under.
struct TxState {
    tx: Mutex<Option<Transaction<'static, Postgres>>>,
    scope: Scope,
}

tokio::task_local! {
    /// The current unit of work's transaction + scope. Bound only inside
    /// [`SqlxUnitOfWork::transaction`]; read by [`with_current_tx`] / [`current_scope`].
    static CURRENT: Arc<TxState>;
}

/// The sqlx connection configuration a composition root hands the adapter.
#[derive(Debug, Clone)]
pub struct SqlxConfig {
    /// The Postgres connection URL (e.g.
    /// `postgres://user@host/db` or `postgres:///db?host=/var/run/postgresql`).
    pub dsn: String,
    /// Maximum pooled connections.
    pub pool_max: u32,
}

/// Builds a Postgres connection [`PgPool`] from `config`. The pool is the handle a
/// [`SqlxUnitOfWork`] / [`SqlxAuditSink`] / [`SqlxIdempotencyStore`] is built from.
pub async fn connect(config: &SqlxConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.pool_max)
        .connect(&config.dsn)
        .await
}

/// Runs `f` with the current unit of work's live `sqlx` transaction (SG1), the
/// **read/write seam a module's repository uses**.
///
/// The scope-based [`UnitOfWork`] passes no handle to the body, so a repository
/// calls this to reach the transaction the surrounding [`SqlxUnitOfWork::transaction`]
/// opened. Every statement run on the `&mut Transaction` joins that transaction and
/// commits or rolls back with it. Returns [`no_ambient_tx`] if called outside one.
///
/// The closure returns a **boxed** future whose lifetime is tied to the borrowed
/// `&mut Transaction` (the `for<'c>` bound) — that higher-ranked lifetime is what
/// lets the body `.await` while holding the transaction reference:
///
/// ```ignore
/// seiva_data_sqlx::with_current_tx(|tx| Box::pin(async move {
///     sqlx::query("INSERT INTO notes (id, title) VALUES ($1, $2)")
///         .bind(id).bind(title)
///         .execute(&mut **tx)
///         .await
///         .map(|_| ())
///         .map_err(Into::into)
/// })).await
/// ```
pub async fn with_current_tx<F, T>(f: F) -> PortResult<T>
where
    F: for<'c> FnOnce(
        &'c mut Transaction<'static, Postgres>,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'c>>,
{
    let state = CURRENT.try_with(Arc::clone).map_err(|_| no_ambient_tx())?;
    let mut guard = state.tx.lock().await;
    let tx = guard.as_mut().ok_or_else(no_ambient_tx)?;
    f(tx).await
}

/// The [`Scope`] the current unit of work was opened for, or `None` outside a
/// transaction. A tenant-aware repository reads it to constrain its own SQL (the
/// sqlx adapter does not auto-filter — the query is the user's).
#[must_use]
pub fn current_scope() -> Option<Scope> {
    CURRENT.try_with(|state| state.scope.clone()).ok()
}

/// The sqlx-backed [`UnitOfWork`]: the scope-based port mapped onto
/// `pool.begin()` / `commit` / `rollback`.
///
/// `transaction` opens a transaction, binds it (with the per-call [`Scope`]) into
/// the [`CURRENT`] task-local, runs the body, and **commits when the body returns
/// `Ok` and rolls back when it returns `Err`**. The body's original (typed)
/// boundary error is returned unchanged — a rollback failure does not mask it — so
/// a status-bearing error survives to the HTTP edge, matching the DORM gateway.
pub struct SqlxUnitOfWork {
    pool: PgPool,
}

impl SqlxUnitOfWork {
    /// Wraps a [`connect`]-built pool as a unit of work.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl UnitOfWork for SqlxUnitOfWork {
    fn transaction<'a, T, F, Fut>(
        &'a self,
        scope: Scope,
        work: F,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'a>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = PortResult<T>> + Send + 'a,
        T: Send + 'a,
    {
        let pool = self.pool.clone();
        Box::pin(async move {
            let tx = pool.begin().await.map_err(boxed)?;
            let state = Arc::new(TxState {
                tx: Mutex::new(Some(tx)),
                scope,
            });
            // Run the body with the transaction bound as the ambient one. The scope
            // clone the body ran under is dropped when it returns, leaving `state`
            // as the sole owner, so we can take the transaction back out below.
            let result = CURRENT.scope(Arc::clone(&state), work()).await;
            let tx = state
                .tx
                .lock()
                .await
                .take()
                .ok_or_else(|| BoxError::from("ambient transaction already consumed"))?;
            match result {
                Ok(value) => {
                    tx.commit().await.map_err(boxed)?;
                    Ok(value)
                }
                Err(err) => {
                    // Best-effort rollback; preserve the caller's ORIGINAL typed
                    // error either way (a rollback error must not mask a 404/409).
                    let _ = tx.rollback().await;
                    Err(err)
                }
            }
        })
    }
}

/// The DDL for the sqlx transactional outbox (`seiva_outbox`), re-exported so a
/// composition root (or an `sqlx migrate` script) can create it. `IF NOT EXISTS`,
/// so running it twice is a no-op.
#[must_use]
pub fn outbox_migrations() -> Vec<String> {
    vec!["CREATE TABLE IF NOT EXISTS seiva_outbox (\
           id bigserial PRIMARY KEY, \
           topic text NOT NULL, \
           payload bytea NOT NULL, \
           occurred_at timestamptz NOT NULL DEFAULT now(), \
           forwarded_at timestamptz)"
        .to_owned()]
}

/// The sqlx-backed [`EventPublisher`]: writes the event to the `seiva_outbox` table
/// **in the current transaction** (the transactional-outbox pattern). Paired with
/// [`SqlxUnitOfWork`], the event and the business change commit together or not at
/// all. It must be called inside a [`UnitOfWork::transaction`] (it reaches the
/// ambient transaction via [`with_current_tx`]); a relay worker forwards the rows
/// to the broker afterwards. The payload is the opaque bytes the application
/// emitted (the interface owns the envelope format).
pub struct SqlxOutboxPublisher;

#[async_trait]
impl EventPublisher for SqlxOutboxPublisher {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()> {
        // Own the inputs so the boxed future borrows only `tx` (the `for<'c>` bound
        // on `with_current_tx` demands the future hold no other external borrow).
        let topic = topic.to_owned();
        let payload = payload.to_vec();
        with_current_tx(move |tx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO seiva_outbox (topic, payload) VALUES ($1, $2)")
                    .bind(topic)
                    .bind(payload)
                    .execute(&mut **tx)
                    .await
                    .map(|_| ())
                    .map_err(boxed)
            })
        })
        .await
    }
}

/// The DDL for the sqlx idempotency table (`seiva_idempotency`, ADR 0011).
/// `IF NOT EXISTS`, so running it twice is a no-op.
#[must_use]
pub fn idempotency_migrations() -> Vec<String> {
    vec!["CREATE TABLE IF NOT EXISTS seiva_idempotency (\
           key text PRIMARY KEY, \
           fingerprint text NOT NULL, \
           response bytea, \
           created_at timestamptz NOT NULL DEFAULT now(), \
           expires_at timestamptz NOT NULL)"
        .to_owned()]
}

/// The sqlx-backed [`IdempotencyStore`] (ADR 0011): the durable, cross-replica
/// guarantee over `seiva_idempotency`. The claim is an atomic
/// `INSERT … ON CONFLICT (key) DO NOTHING` — a database CAS, not an in-memory lock
/// — so two replicas racing one key resolve to exactly one
/// [`Fresh`](IdempotencyOutcome::Fresh).
///
/// Call `begin`/`finish` **inside** a [`SqlxUnitOfWork::transaction`]: the
/// reservation then commits in the same transaction as the domain mutation and the
/// outbox write, so a committed key always has its effect and a rollback un-claims
/// it. `purge_expired` runs on its own pooled connection (a scheduled worker, not
/// a request transaction).
pub struct SqlxIdempotencyStore {
    pool: PgPool,
}

impl SqlxIdempotencyStore {
    /// Wraps a [`connect`]-built pool as an idempotency store.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IdempotencyStore for SqlxIdempotencyStore {
    async fn begin(
        &self,
        scoped_key: &str,
        fingerprint: &str,
        ttl: Duration,
    ) -> PortResult<IdempotencyOutcome> {
        // Seconds as f64 for `make_interval(secs => …)`; saturates for an
        // absurdly long ttl, which only makes the record live longer.
        let ttl_secs = ttl.as_secs_f64();
        let scoped_key = scoped_key.to_owned();
        let fingerprint = fingerprint.to_owned();
        with_current_tx(move |tx| {
            Box::pin(async move {
                // Atomic claim: only the first inserter affects a row.
                let claimed = sqlx::query(
                    "INSERT INTO seiva_idempotency (key, fingerprint, expires_at) \
                     VALUES ($1, $2, now() + make_interval(secs => $3)) \
                     ON CONFLICT (key) DO NOTHING",
                )
                .bind(scoped_key.as_str())
                .bind(fingerprint.as_str())
                .bind(ttl_secs)
                .execute(&mut **tx)
                .await
                .map_err(boxed)?;
                if claimed.rows_affected() == 1 {
                    return Ok(IdempotencyOutcome::Fresh);
                }
                // The key already exists: inspect it. A different fingerprint is a
                // Mismatch; a null response is still InProgress; otherwise Replay.
                let (stored_fp, response): (String, Option<Vec<u8>>) = sqlx::query_as(
                    "SELECT fingerprint, response FROM seiva_idempotency WHERE key = $1",
                )
                .bind(scoped_key.as_str())
                .fetch_one(&mut **tx)
                .await
                .map_err(boxed)?;
                if stored_fp != fingerprint {
                    return Ok(IdempotencyOutcome::Mismatch);
                }
                Ok(match response {
                    Some(bytes) => IdempotencyOutcome::Replay(bytes),
                    None => IdempotencyOutcome::InProgress,
                })
            })
        })
        .await
    }

    async fn finish(&self, scoped_key: &str, response: &[u8]) -> PortResult<()> {
        let scoped_key = scoped_key.to_owned();
        let response = response.to_vec();
        with_current_tx(move |tx| {
            Box::pin(async move {
                sqlx::query("UPDATE seiva_idempotency SET response = $2 WHERE key = $1")
                    .bind(scoped_key)
                    .bind(response)
                    .execute(&mut **tx)
                    .await
                    .map(|_| ())
                    .map_err(boxed)
            })
        })
        .await
    }

    async fn abort(&self, scoped_key: &str) -> PortResult<()> {
        // Release only an in-flight claim (null response); a finished record is a
        // real Replay and must survive.
        let scoped_key = scoped_key.to_owned();
        with_current_tx(move |tx| {
            Box::pin(async move {
                sqlx::query("DELETE FROM seiva_idempotency WHERE key = $1 AND response IS NULL")
                    .bind(scoped_key)
                    .execute(&mut **tx)
                    .await
                    .map(|_| ())
                    .map_err(boxed)
            })
        })
        .await
    }

    async fn purge_expired(&self) -> PortResult<u64> {
        let result = sqlx::query("DELETE FROM seiva_idempotency WHERE expires_at < now()")
            .execute(&self.pool)
            .await
            .map_err(boxed)?;
        Ok(result.rows_affected())
    }
}

/// The DDL for the sqlx audit table (`seiva_audit`, ADR 0013). `IF NOT EXISTS`.
#[must_use]
pub fn audit_migrations() -> Vec<String> {
    vec!["CREATE TABLE IF NOT EXISTS seiva_audit (\
           id bigserial PRIMARY KEY, \
           at timestamptz NOT NULL DEFAULT now(), \
           actor text NOT NULL, \
           tenant text, \
           operation text NOT NULL, \
           object text, \
           outcome text NOT NULL, \
           request_id text, \
           details jsonb NOT NULL DEFAULT '{}')"
        .to_owned()]
}

/// The sqlx-backed [`AuditSink`] (ADR 0013): appends each entry to `seiva_audit`.
///
/// Like the DORM sink, audit does **not** join the request's transaction — it
/// records on its own pooled connection (the audit middleware runs above any
/// transaction and must capture denied mutations too). Entries arrive already
/// redacted.
pub struct SqlxAuditSink {
    pool: PgPool,
}

impl SqlxAuditSink {
    /// Wraps a [`connect`]-built pool as an audit sink.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditSink for SqlxAuditSink {
    async fn record(&self, entry: &AuditEntry) -> PortResult<()> {
        let details = serde_json::Value::Object(
            entry
                .details
                .iter()
                .cloned()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect(),
        );
        sqlx::query(
            "INSERT INTO seiva_audit \
             (actor, tenant, operation, object, outcome, request_id, details) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&entry.actor)
        .bind(&entry.tenant)
        .bind(&entry.operation)
        .bind(&entry.object)
        .bind(&entry.outcome)
        .bind(&entry.request_id)
        .bind(sqlx::types::Json(details))
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(boxed)
    }
}

/// The sqlx-backed [`AuditLog`] (ADR 0013): the **read** twin of [`SqlxAuditSink`],
/// returning the persisted `seiva_audit` entries matching an [`AuditQuery`],
/// **newest first** and **bounded** by the query's
/// [`effective_limit`](AuditQuery::effective_limit).
///
/// Like the sink it reads on its own pooled connection (the admin read path runs
/// above any request transaction). The filters are AND-combined and **bound** as
/// parameters — never interpolated — so `actor`/`tenant`/`object`/`outcome` match
/// exactly (an unset or empty filter matches anything, exactly as
/// [`AuditQuery::matches`] defines it). Ordering is by the `bigserial` primary key
/// descending, so "newest first" is the insertion order the sink produced, and the
/// `LIMIT` is the bounded effective limit, so a history view can never ask for an
/// unbounded page.
pub struct SqlxAuditLog {
    pool: PgPool,
}

impl SqlxAuditLog {
    /// Wraps a [`connect`]-built pool as an audit log reader.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditLog for SqlxAuditLog {
    async fn query(&self, query: &AuditQuery) -> PortResult<Vec<AuditEntry>> {
        use sqlx::Row;

        // Build the WHERE clause from only the *set* filters (a `None` or empty
        // filter matches anything — the same rule `AuditQuery::matches` applies),
        // binding each as a parameter so a value can never be interpolated into SQL.
        let mut conditions: Vec<String> = Vec::new();
        let mut binds: Vec<String> = Vec::new();
        let mut push = |column: &str, filter: &Option<String>| {
            if let Some(wanted) = filter.as_deref().filter(|s| !s.is_empty()) {
                binds.push(wanted.to_owned());
                conditions.push(format!("{column} = ${}", binds.len()));
            }
        };
        push("actor", &query.actor);
        push("tenant", &query.tenant);
        push("object", &query.object);
        push("outcome", &query.outcome);

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        // The limit is server-controlled and bounded; bind it too (as i64) rather
        // than interpolate, keeping the whole statement parameterized.
        let limit = i64::from(query.effective_limit());
        let sql = format!(
            "SELECT actor, tenant, operation, object, outcome, request_id, details \
             FROM seiva_audit{where_clause} ORDER BY id DESC LIMIT ${}",
            binds.len() + 1
        );

        // Only framework-owned column names and `$n` placeholders are interpolated
        // (every filter VALUE is bound above); `AssertSqlSafe` is sqlx 0.9's opt-in
        // for a non-`'static` query string.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for bind in binds {
            q = q.bind(bind);
        }
        q = q.bind(limit);
        let rows = q.fetch_all(&self.pool).await.map_err(boxed)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let actor: String = row.try_get("actor").map_err(boxed)?;
            let tenant: Option<String> = row.try_get("tenant").map_err(boxed)?;
            let operation: String = row.try_get("operation").map_err(boxed)?;
            let object: Option<String> = row.try_get("object").map_err(boxed)?;
            let outcome: String = row.try_get("outcome").map_err(boxed)?;
            let request_id: Option<String> = row.try_get("request_id").map_err(boxed)?;
            let details: serde_json::Value = row.try_get("details").map_err(boxed)?;

            // Reconstruct through the builder, never a literal (`AuditEntry` is
            // `#[non_exhaustive]`). `details` comes back as the redacted JSON object
            // the sink stored; rehydrate it as the `(name, value)` detail pairs.
            let mut entry = AuditEntry::new(actor, operation, outcome)
                .in_tenant(tenant)
                .on_object(object)
                .with_request_id(request_id);
            if let serde_json::Value::Object(map) = details {
                for (name, value) in map {
                    let value = match value {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    entry = entry.with_detail(name, value);
                }
            }
            entries.push(entry);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The DDL helpers are non-empty and idempotent-looking (offline sanity).
    #[test]
    fn migrations_are_present_and_idempotent() {
        for stmt in outbox_migrations()
            .iter()
            .chain(&idempotency_migrations())
            .chain(&audit_migrations())
        {
            assert!(
                stmt.contains("IF NOT EXISTS"),
                "DDL must be idempotent: {stmt}"
            );
        }
    }

    // The ports fail cleanly when driven with no ambient transaction, rather than
    // panicking — the task-local is simply unbound here.
    #[tokio::test]
    async fn ports_require_an_ambient_transaction() {
        assert!(current_scope().is_none());
        let published = SqlxOutboxPublisher.publish("t", b"x").await;
        assert!(
            published.is_err(),
            "publish outside a transaction must error"
        );
        let claimed = with_current_tx(|_tx| Box::pin(async { Ok::<(), BoxError>(()) })).await;
        assert!(
            claimed.is_err(),
            "with_current_tx outside a transaction must error"
        );
    }
}
