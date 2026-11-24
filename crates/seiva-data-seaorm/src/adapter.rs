//! The SeaORM adapter proper, gated behind the `seaorm` feature.

use async_trait::async_trait;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection,
    DatabaseTransaction, Statement, TransactionTrait, Value,
};
use seiva_ports::{
    AuditEntry, AuditLog, AuditQuery, AuditSink, BoxError, EventPublisher, IdempotencyOutcome,
    IdempotencyStore, PortResult, Scope, UnitOfWork,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Boxes any concrete error as the port boundary's opaque [`BoxError`].
fn boxed<E: std::error::Error + Send + Sync + 'static>(e: E) -> BoxError {
    Box::new(e)
}

/// The error a port returns when driven outside a [`UnitOfWork::transaction`].
fn no_ambient_tx() -> BoxError {
    "no ambient SeaORM transaction in scope: call this inside UnitOfWork::transaction".into()
}

/// State bound into the task-local for one unit of work: the live transaction (an
/// `Option` so it can be taken back out to commit/roll back) and its [`Scope`].
struct TxState {
    tx: Mutex<Option<DatabaseTransaction>>,
    scope: Scope,
}

tokio::task_local! {
    /// The current unit of work's transaction + scope. Bound only inside
    /// [`SeaOrmUnitOfWork::transaction`]; read by [`with_current_tx`] / [`current_scope`].
    static CURRENT: Arc<TxState>;
}

/// The SeaORM connection configuration a composition root hands the adapter.
#[derive(Debug, Clone)]
pub struct SeaOrmConfig {
    /// The Postgres connection URL (e.g.
    /// `postgres://user@host/db` or `postgres://user@%2Fvar%2Frun%2Fpostgresql/db`).
    pub dsn: String,
    /// Maximum pooled connections.
    pub pool_max: u32,
}

/// Builds a SeaORM [`DatabaseConnection`] from `config`.
pub async fn connect(config: &SeaOrmConfig) -> Result<DatabaseConnection, sea_orm::DbErr> {
    let mut options = ConnectOptions::new(config.dsn.clone());
    options.max_connections(config.pool_max);
    Database::connect(options).await
}

/// Runs `f` with the current unit of work's live SeaORM transaction (SG1) — the
/// read/write seam a module's repository uses.
///
/// The scope-based [`UnitOfWork`] passes no handle to the body, so a repository
/// calls this to reach the transaction the surrounding
/// [`SeaOrmUnitOfWork::transaction`] opened. The closure returns a **boxed** future
/// whose lifetime is tied to the borrowed `&DatabaseTransaction` (the `for<'c>`
/// bound), which is what lets the body `.await` while holding the reference.
/// Returns [`no_ambient_tx`] if called outside a transaction.
///
/// ```ignore
/// seiva_data_seaorm::with_current_tx(|txn| Box::pin(async move {
///     use sea_orm::{ConnectionTrait, Statement, DatabaseBackend, Value};
///     txn.execute_raw(Statement::from_sql_and_values(
///         DatabaseBackend::Postgres,
///         "INSERT INTO notes (id, title) VALUES ($1, $2)",
///         vec![Value::from(id), Value::from(title)],
///     )).await.map(|_| ()).map_err(Into::into)
/// })).await
/// ```
pub async fn with_current_tx<F, T>(f: F) -> PortResult<T>
where
    F: for<'c> FnOnce(
        &'c DatabaseTransaction,
    ) -> Pin<Box<dyn Future<Output = PortResult<T>> + Send + 'c>>,
{
    let state = CURRENT.try_with(Arc::clone).map_err(|_| no_ambient_tx())?;
    let guard = state.tx.lock().await;
    let txn = guard.as_ref().ok_or_else(no_ambient_tx)?;
    f(txn).await
}

/// The [`Scope`] the current unit of work was opened for, or `None` outside a
/// transaction. A tenant-aware repository reads it to constrain its own SQL.
#[must_use]
pub fn current_scope() -> Option<Scope> {
    CURRENT.try_with(|state| state.scope.clone()).ok()
}

/// The SeaORM-backed [`UnitOfWork`]: the scope-based port mapped onto
/// `conn.begin()` / `commit` / `rollback`.
///
/// `transaction` opens a transaction, binds it (with the per-call [`Scope`]) into
/// the [`CURRENT`] task-local, runs the body, and **commits when the body returns
/// `Ok` and rolls back when it returns `Err`**. The body's original (typed) error
/// is returned unchanged — a rollback failure does not mask it — so a status-bearing
/// error survives to the HTTP edge, matching the DORM gateway.
pub struct SeaOrmUnitOfWork {
    conn: DatabaseConnection,
}

impl SeaOrmUnitOfWork {
    /// Wraps a [`connect`]-built connection as a unit of work.
    #[must_use]
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

impl UnitOfWork for SeaOrmUnitOfWork {
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
        let conn = self.conn.clone();
        Box::pin(async move {
            let txn = conn.begin().await.map_err(boxed)?;
            let state = Arc::new(TxState {
                tx: Mutex::new(Some(txn)),
                scope,
            });
            let result = CURRENT.scope(Arc::clone(&state), work()).await;
            let txn = state
                .tx
                .lock()
                .await
                .take()
                .ok_or_else(|| BoxError::from("ambient transaction already consumed"))?;
            match result {
                Ok(value) => {
                    txn.commit().await.map_err(boxed)?;
                    Ok(value)
                }
                Err(err) => {
                    let _ = txn.rollback().await;
                    Err(err)
                }
            }
        })
    }
}

/// The DDL for the SeaORM transactional outbox (`seiva_outbox`). `IF NOT EXISTS`.
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

/// The SeaORM-backed [`EventPublisher`]: writes the event to `seiva_outbox` **in
/// the current transaction** (the transactional-outbox pattern). Paired with
/// [`SeaOrmUnitOfWork`], the event and the business change commit together. Must be
/// called inside a [`UnitOfWork::transaction`]; a relay worker forwards the rows.
pub struct SeaOrmOutboxPublisher;

#[async_trait]
impl EventPublisher for SeaOrmOutboxPublisher {
    async fn publish(&self, topic: &str, payload: &[u8]) -> PortResult<()> {
        let topic = topic.to_owned();
        let payload = payload.to_vec();
        with_current_tx(move |txn| {
            Box::pin(async move {
                txn.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "INSERT INTO seiva_outbox (topic, payload) VALUES ($1, $2)",
                    vec![Value::String(Some(topic)), Value::Bytes(Some(payload))],
                ))
                .await
                .map(|_| ())
                .map_err(boxed)
            })
        })
        .await
    }
}

/// The DDL for the SeaORM idempotency table (`seiva_idempotency`, ADR 0011).
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

/// The SeaORM-backed [`IdempotencyStore`] (ADR 0011): the durable, cross-replica
/// guarantee over `seiva_idempotency`. The claim is an atomic
/// `INSERT … ON CONFLICT (key) DO NOTHING`, so two replicas racing one key resolve
/// to exactly one [`Fresh`](IdempotencyOutcome::Fresh). Call `begin`/`finish`
/// **inside** a [`SeaOrmUnitOfWork::transaction`]; `purge_expired` runs on its own
/// connection.
pub struct SeaOrmIdempotencyStore {
    conn: DatabaseConnection,
}

impl SeaOrmIdempotencyStore {
    /// Wraps a [`connect`]-built connection as an idempotency store.
    #[must_use]
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl IdempotencyStore for SeaOrmIdempotencyStore {
    async fn begin(
        &self,
        scoped_key: &str,
        fingerprint: &str,
        ttl: Duration,
    ) -> PortResult<IdempotencyOutcome> {
        let ttl_secs = ttl.as_secs_f64();
        let scoped_key = scoped_key.to_owned();
        let fingerprint = fingerprint.to_owned();
        with_current_tx(move |txn| {
            Box::pin(async move {
                // Atomic claim: only the first inserter affects a row.
                let claimed = txn
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "INSERT INTO seiva_idempotency (key, fingerprint, expires_at) \
                         VALUES ($1, $2, now() + make_interval(secs => $3)) \
                         ON CONFLICT (key) DO NOTHING",
                        vec![
                            Value::String(Some(scoped_key.clone())),
                            Value::String(Some(fingerprint.clone())),
                            Value::Double(Some(ttl_secs)),
                        ],
                    ))
                    .await
                    .map_err(boxed)?;
                if claimed.rows_affected() == 1 {
                    return Ok(IdempotencyOutcome::Fresh);
                }
                // The key already exists: inspect it.
                let row = txn
                    .query_one_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "SELECT fingerprint, response FROM seiva_idempotency WHERE key = $1",
                        vec![Value::String(Some(scoped_key.clone()))],
                    ))
                    .await
                    .map_err(boxed)?
                    .ok_or_else(|| BoxError::from("idempotency row vanished after conflict"))?;
                let stored_fp: String = row
                    .try_get_by_index(0)
                    .map_err(|e| BoxError::from(e.to_string()))?;
                let response: Option<Vec<u8>> = row
                    .try_get_by_index(1)
                    .map_err(|e| BoxError::from(e.to_string()))?;
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
        with_current_tx(move |txn| {
            Box::pin(async move {
                txn.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE seiva_idempotency SET response = $2 WHERE key = $1",
                    vec![
                        Value::String(Some(scoped_key)),
                        Value::Bytes(Some(response)),
                    ],
                ))
                .await
                .map(|_| ())
                .map_err(boxed)
            })
        })
        .await
    }

    async fn abort(&self, scoped_key: &str) -> PortResult<()> {
        let scoped_key = scoped_key.to_owned();
        with_current_tx(move |txn| {
            Box::pin(async move {
                txn.execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "DELETE FROM seiva_idempotency WHERE key = $1 AND response IS NULL",
                    vec![Value::String(Some(scoped_key))],
                ))
                .await
                .map(|_| ())
                .map_err(boxed)
            })
        })
        .await
    }

    async fn purge_expired(&self) -> PortResult<u64> {
        let result = self
            .conn
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "DELETE FROM seiva_idempotency WHERE expires_at < now()",
            ))
            .await
            .map_err(boxed)?;
        Ok(result.rows_affected())
    }
}

/// The DDL for the SeaORM audit table (`seiva_audit`, ADR 0013). `IF NOT EXISTS`.
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

/// The SeaORM-backed [`AuditSink`] (ADR 0013): appends each entry to `seiva_audit`
/// on its own connection (the audit middleware runs above any transaction). Entries
/// arrive already redacted. The JSON `details` is cast in SQL (`$7::jsonb`) so the
/// adapter needs no SeaORM JSON feature.
pub struct SeaOrmAuditSink {
    conn: DatabaseConnection,
}

impl SeaOrmAuditSink {
    /// Wraps a [`connect`]-built connection as an audit sink.
    #[must_use]
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

/// Builds a nullable text [`Value`] from an `Option<String>`.
fn opt_text(value: &Option<String>) -> Value {
    Value::String(value.clone())
}

#[async_trait]
impl AuditSink for SeaOrmAuditSink {
    async fn record(&self, entry: &AuditEntry) -> PortResult<()> {
        let details = serde_json::Value::Object(
            entry
                .details
                .iter()
                .cloned()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect(),
        );
        let details_text = serde_json::to_string(&details).map_err(boxed)?;
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO seiva_audit \
                 (actor, tenant, operation, object, outcome, request_id, details) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)",
                vec![
                    Value::String(Some(entry.actor.clone())),
                    opt_text(&entry.tenant),
                    Value::String(Some(entry.operation.clone())),
                    opt_text(&entry.object),
                    Value::String(Some(entry.outcome.clone())),
                    opt_text(&entry.request_id),
                    Value::String(Some(details_text)),
                ],
            ))
            .await
            .map(|_| ())
            .map_err(boxed)
    }
}

/// The SeaORM-backed [`AuditLog`] (ADR 0013): the **read** twin of
/// [`SeaOrmAuditSink`], returning the persisted `seiva_audit` entries matching an
/// [`AuditQuery`], **newest first** and **bounded** by the query's
/// [`effective_limit`](AuditQuery::effective_limit).
///
/// Like the sink it reads on its own connection (the admin read path runs above any
/// request transaction). The filters are AND-combined and **bound** as parameters —
/// never interpolated — so `actor`/`tenant`/`object`/`outcome` match exactly (an
/// unset or empty filter matches anything, exactly as [`AuditQuery::matches`]
/// defines it). Ordering is by the `bigserial` primary key descending, so "newest
/// first" is the insertion order the sink produced, and the `LIMIT` is the bounded
/// effective limit, so a history view can never ask for an unbounded page. The
/// JSON `details` is read back as text (`details::text`) and reparsed, so the
/// adapter needs no SeaORM JSON feature — symmetric with how the sink casts it.
pub struct SeaOrmAuditLog {
    conn: DatabaseConnection,
}

impl SeaOrmAuditLog {
    /// Wraps a [`connect`]-built connection as an audit log reader.
    #[must_use]
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl AuditLog for SeaOrmAuditLog {
    async fn query(&self, query: &AuditQuery) -> PortResult<Vec<AuditEntry>> {
        // Build the WHERE clause from only the *set* filters (a `None` or empty
        // filter matches anything — the same rule `AuditQuery::matches` applies),
        // binding each as a parameter so a value can never be interpolated into SQL.
        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut push = |column: &str, filter: &Option<String>| {
            if let Some(wanted) = filter.as_deref().filter(|s| !s.is_empty()) {
                params.push(Value::String(Some(wanted.to_owned())));
                conditions.push(format!("{column} = ${}", params.len()));
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
        params.push(Value::BigInt(Some(i64::from(query.effective_limit()))));
        let sql = format!(
            "SELECT actor, tenant, operation, object, outcome, request_id, details::text \
             FROM seiva_audit{where_clause} ORDER BY id DESC LIMIT ${}",
            params.len()
        );

        let rows = self
            .conn
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                sql,
                params,
            ))
            .await
            .map_err(boxed)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let actor: String = row.try_get_by_index(0).map_err(boxed)?;
            let tenant: Option<String> = row.try_get_by_index(1).map_err(boxed)?;
            let operation: String = row.try_get_by_index(2).map_err(boxed)?;
            let object: Option<String> = row.try_get_by_index(3).map_err(boxed)?;
            let outcome: String = row.try_get_by_index(4).map_err(boxed)?;
            let request_id: Option<String> = row.try_get_by_index(5).map_err(boxed)?;
            let details_text: String = row.try_get_by_index(6).map_err(boxed)?;

            // Reconstruct through the builder, never a literal (`AuditEntry` is
            // `#[non_exhaustive]`). `details` comes back as the redacted JSON object
            // the sink stored (as text); rehydrate it as the `(name, value)` pairs.
            let mut entry = AuditEntry::new(actor, operation, outcome)
                .in_tenant(tenant)
                .on_object(object)
                .with_request_id(request_id);
            let details: serde_json::Value = serde_json::from_str(&details_text).map_err(boxed)?;
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

    #[tokio::test]
    async fn ports_require_an_ambient_transaction() {
        assert!(current_scope().is_none());
        let published = SeaOrmOutboxPublisher.publish("t", b"x").await;
        assert!(
            published.is_err(),
            "publish outside a transaction must error"
        );
        let claimed = with_current_tx(|_txn| Box::pin(async { Ok::<(), BoxError>(()) })).await;
        assert!(
            claimed.is_err(),
            "with_current_tx outside a transaction must error"
        );
    }
}
