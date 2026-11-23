//! The DORM-backed [`DeliveryLedger`] (ADR 0033): the durable source of truth the
//! webhook [`DeliveryEngine`](seiva_webhooks::delivery::DeliveryEngine) drives,
//! persisted to one PostgreSQL table.
//!
//! The decisive property is **where each operation runs its SQL**:
//!
//! - [`enqueue`](DeliveryLedger::enqueue) runs on the **ambient** session — it
//!   issues its `INSERT` through [`dorm_core::db::run`] without opening a session
//!   of its own, so when the [`DeliverySubscriber`](seiva_webhook_delivery) enqueues
//!   *inside* a handler running under [`DormUnitOfWork::transaction`](super::DormUnitOfWork)
//!   (together with the inbox claim+mark of ADR 0011), the owed row commits in the
//!   **same transaction** as the event is consumed. A committed event always has
//!   its deliveries owed, and a rolled-back one leaves neither the row nor the
//!   inbox mark — the atomicity the e2e proves.
//! - [`claim_due`](DeliveryLedger::claim_due), [`record`](DeliveryLedger::record)
//!   and [`get`](DeliveryLedger::get) are driven by the delivery **worker**, which
//!   runs *outside* any business transaction, so each opens its **own** session
//!   ([`with_session`](dorm_core::db::with_session)). `claim_due` leases a batch
//!   atomically with a single `UPDATE … FOR UPDATE SKIP LOCKED … RETURNING`, so N
//!   replicas draining the table never claim the same row twice.
//!
//! The table is driven with **raw SQL** (like [`DormAuditSink`](super::DormAuditSink)
//! and [`DormIdempotencyStore`](super::DormIdempotencyStore)), not as a
//! `tenant_scoped` DORM model: a delivery worker legitimately drains every
//! tenant's rows, so the row carries its owning tenant in a column (written from
//! the [`NewDelivery`] scope, fail-closed on a missing or unmapped scope) rather
//! than relying on the task-local tenant predicate.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dorm_core::db::{with_session, Database};
use dorm_core::value::Value;
use seiva_ports::Scope;
use seiva_webhooks::delivery::{
    Delivery, DeliveryId, DeliveryLedger, DeliveryStatus, DeliveryUpdate, LedgerError,
    LedgerResult, NewDelivery,
};

/// The delivery table's name — one row per `(subscription, event)` delivery.
const TABLE: &str = "seiva_webhook_deliveries";

/// The column list, in the one order both reads and `RETURNING` decode against.
const COLUMNS: &str = "id, subscription, event_id, endpoint, payload, tenant, \
     status, attempt, next_attempt_at, last_http_status, last_response_excerpt";

/// The DDL for the delivery table (and its due-work index), re-exported so a
/// composition root or migration can create it. `CREATE … IF NOT EXISTS`, so
/// running it twice is a no-op.
///
/// The `(status, next_attempt_at)` index is what makes [`claim_due`](DeliveryLedger::claim_due)'s
/// due-row scan cheap once the table holds a backlog.
#[must_use]
pub fn delivery_migrations() -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE IF NOT EXISTS {TABLE} (\
               id TEXT PRIMARY KEY, \
               subscription TEXT NOT NULL, \
               event_id TEXT NOT NULL, \
               endpoint TEXT NOT NULL, \
               payload BYTEA NOT NULL, \
               tenant TEXT, \
               status TEXT NOT NULL, \
               attempt INTEGER NOT NULL, \
               next_attempt_at TIMESTAMPTZ NOT NULL, \
               lease_until TIMESTAMPTZ NOT NULL, \
               last_http_status INTEGER, \
               last_response_excerpt TEXT)"
        ),
        format!("CREATE INDEX IF NOT EXISTS {TABLE}_due ON {TABLE} (status, next_attempt_at)"),
    ]
}

/// The DORM-backed [`DeliveryLedger`]. Cheap to clone (an `Arc` inside), so the
/// subscriber and the engine hold handles onto the same table.
///
/// The handle is only ever used by the worker operations
/// ([`claim_due`](DeliveryLedger::claim_due)/[`record`](DeliveryLedger::record)/[`get`](DeliveryLedger::get))
/// to open their own session; [`enqueue`](DeliveryLedger::enqueue) deliberately
/// ignores it and joins the ambient transaction instead.
#[derive(Clone)]
pub struct DormDeliveryLedger {
    db: Arc<Database>,
}

impl DormDeliveryLedger {
    /// Wrap a [`connect`](super::connect)-built handle as a delivery ledger.
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db: Arc::new(db) }
    }

    /// Wrap an already-shared handle (so the worker and the rest of a service can
    /// share one pool).
    #[must_use]
    pub fn from_shared(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// A `SystemTime` as the `TIMESTAMPTZ` bind value DORM expects.
fn to_timestamp(t: SystemTime) -> Value {
    Value::Timestamp(DateTime::<Utc>::from(t))
}

/// The `tenant` column bind for a scope. A [`Scope::Tenant`] stores its id; the
/// audited [`Scope::Global`] stores `NULL`; any **future** scope variant fails
/// closed (`#[non_exhaustive]`) rather than silently persist a tenant-less row.
fn tenant_bind(scope: &Scope) -> LedgerResult<Value> {
    match scope {
        Scope::Tenant(tenant) => Ok(Value::Text(tenant.as_str().to_owned())),
        Scope::Global => Ok(Value::Null),
        _ => Err(LedgerError::Backend(
            "unmapped Scope variant reached the DORM delivery ledger (fail-closed)".to_owned(),
        )),
    }
}

/// Parse a stored status token back into the lifecycle enum.
fn status_from_str(token: &str) -> LedgerResult<DeliveryStatus> {
    Ok(match token {
        "pending" => DeliveryStatus::Pending,
        "in_flight" => DeliveryStatus::InFlight,
        "delivered" => DeliveryStatus::Delivered,
        "failed" => DeliveryStatus::Failed,
        "dead" => DeliveryStatus::Dead,
        other => {
            return Err(LedgerError::Backend(format!(
                "unknown delivery status {other:?} in the ledger"
            )))
        }
    })
}

/// Decode one `COLUMNS`-ordered row into a [`Delivery`].
fn row_to_delivery(row: &dorm_core::row::Row) -> LedgerResult<Delivery> {
    let backend = |e: dorm_core::Error| LedgerError::Backend(e.to_string());

    let id: String = row.try_get(0).map_err(backend)?;
    let subscription: String = row.try_get(1).map_err(backend)?;
    let event_id: String = row.try_get(2).map_err(backend)?;
    let endpoint: String = row.try_get(3).map_err(backend)?;
    let payload: Vec<u8> = row.try_get(4).map_err(backend)?;
    let tenant: Option<String> = row.try_get(5).map_err(backend)?;
    let status: String = row.try_get(6).map_err(backend)?;
    let attempt: i32 = row.try_get(7).map_err(backend)?;
    let next_attempt_at: DateTime<Utc> = row.try_get(8).map_err(backend)?;
    let last_http_status: Option<i32> = row.try_get(9).map_err(backend)?;
    let last_response_excerpt: Option<String> = row.try_get(10).map_err(backend)?;

    // A stored tenant is always non-empty (we only ever write a non-empty id); a
    // NULL tenant is the audited global scope.
    let scope = match tenant {
        Some(tenant) => Scope::tenant(tenant).ok_or_else(|| {
            LedgerError::Backend("empty tenant stored in a delivery row".to_owned())
        })?,
        None => Scope::Global,
    };

    Ok(Delivery {
        id: DeliveryId::new(id),
        subscription,
        event_id,
        endpoint,
        payload,
        scope,
        status: status_from_str(&status)?,
        attempt: u32::try_from(attempt)
            .map_err(|_| LedgerError::Backend("negative attempt in a delivery row".to_owned()))?,
        next_attempt_at: SystemTime::from(next_attempt_at),
        last_http_status: last_http_status.map(|n| n as u16),
        last_response_excerpt,
    })
}

#[async_trait]
impl DeliveryLedger for DormDeliveryLedger {
    async fn enqueue(&self, new: NewDelivery) -> LedgerResult<()> {
        // Fail closed on a missing tenant scope (ADR 0019): a tenant-less delivery
        // is a bug, not a shared-root send.
        let scope = new.scope.ok_or(LedgerError::MissingScope)?;
        let tenant = tenant_bind(&scope)?;

        // Idempotent on id: a replayed outbox drain (or a redelivered event, whose
        // id is deterministic) never duplicates a delivery. A due-now row mirrors
        // the in-memory double: `next_attempt_at = epoch` is `<= now` for any real
        // clock, so the next tick claims it.
        let params = [
            Value::Text(new.id.as_str().to_owned()),
            Value::Text(new.subscription),
            Value::Text(new.event_id),
            Value::Text(new.endpoint),
            Value::Bytes(new.payload),
            tenant,
            Value::Text(DeliveryStatus::Pending.as_str().to_owned()),
            Value::I32(0),
            to_timestamp(UNIX_EPOCH),
            to_timestamp(UNIX_EPOCH),
            Value::Null,
            Value::Null,
        ];
        // NO `with_session` here: the INSERT joins whatever session/transaction is
        // ambient — the subscriber's `UnitOfWork::transaction` — so the owed row
        // commits (or rolls back) atomically with the inbox mark.
        dorm_core::db::run(
            &format!(
                "INSERT INTO {TABLE} \
                 (id, subscription, event_id, endpoint, payload, tenant, status, \
                  attempt, next_attempt_at, lease_until, last_http_status, last_response_excerpt) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) \
                 ON CONFLICT (id) DO NOTHING"
            ),
            &params,
        )
        .await
        .map(|_| ())
        .map_err(|e| LedgerError::Backend(e.to_string()))
    }

    async fn claim_due(
        &self,
        now: SystemTime,
        max: usize,
        lease: Duration,
    ) -> LedgerResult<Vec<Delivery>> {
        // A single statement atomically leases the batch: the subquery selects the
        // due rows (pending-and-arrived, or in-flight with a lapsed lease — a
        // crashed worker's orphan), locks them with FOR UPDATE SKIP LOCKED so a
        // concurrent worker skips rather than blocks, and the outer UPDATE flips
        // them to in_flight under a fresh lease, RETURNING the leased rows.
        let sql = format!(
            "UPDATE {TABLE} SET status = 'in_flight', lease_until = $1 \
             WHERE id IN (\
               SELECT id FROM {TABLE} \
               WHERE (status = 'pending' AND next_attempt_at <= $2) \
                  OR (status = 'in_flight' AND lease_until <= $2) \
               ORDER BY next_attempt_at, id \
               LIMIT $3 \
               FOR UPDATE SKIP LOCKED) \
             RETURNING {COLUMNS}"
        );
        let params = [
            to_timestamp(now + lease),
            to_timestamp(now),
            Value::I64(i64::try_from(max).unwrap_or(i64::MAX)),
        ];
        let db = Arc::clone(&self.db);
        let rows = with_session(db, async move { dorm_core::db::sql(&sql, &params).await })
            .await
            .map_err(|e| LedgerError::Backend(e.to_string()))?;
        rows.iter().map(row_to_delivery).collect()
    }

    async fn record(&self, id: &DeliveryId, update: DeliveryUpdate) -> LedgerResult<()> {
        // A terminal row holds no lease and a rescheduled one is only claimable at
        // its next_attempt_at, so lease_until tracks next_attempt_at (mirroring the
        // in-memory double).
        let sql = format!(
            "UPDATE {TABLE} SET status = $1, attempt = $2, next_attempt_at = $3, \
             lease_until = $3, last_http_status = $4, last_response_excerpt = $5 \
             WHERE id = $6"
        );
        let params = [
            Value::Text(update.status.as_str().to_owned()),
            Value::I32(i32::try_from(update.attempt).unwrap_or(i32::MAX)),
            to_timestamp(update.next_attempt_at),
            update
                .last_http_status
                .map_or(Value::Null, |s| Value::I32(i32::from(s))),
            update
                .last_response_excerpt
                .map_or(Value::Null, Value::Text),
            Value::Text(id.as_str().to_owned()),
        ];
        let db = Arc::clone(&self.db);
        let affected = with_session(db, async move { dorm_core::db::run(&sql, &params).await })
            .await
            .map_err(|e| LedgerError::Backend(e.to_string()))?;
        if affected == 0 {
            return Err(LedgerError::NotFound);
        }
        Ok(())
    }

    async fn get(&self, id: &DeliveryId) -> LedgerResult<Option<Delivery>> {
        let sql = format!("SELECT {COLUMNS} FROM {TABLE} WHERE id = $1");
        let params = [Value::Text(id.as_str().to_owned())];
        let db = Arc::clone(&self.db);
        let rows = with_session(db, async move { dorm_core::db::sql(&sql, &params).await })
            .await
            .map_err(|e| LedgerError::Backend(e.to_string()))?;
        rows.first().map(row_to_delivery).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_token() {
        for status in [
            DeliveryStatus::Pending,
            DeliveryStatus::InFlight,
            DeliveryStatus::Delivered,
            DeliveryStatus::Failed,
            DeliveryStatus::Dead,
        ] {
            assert_eq!(status_from_str(status.as_str()).unwrap(), status);
        }
        assert!(status_from_str("bogus").is_err());
    }

    #[test]
    fn tenant_bind_maps_scope_fail_closed() {
        let tenant = Scope::tenant("acme").unwrap();
        assert_eq!(
            tenant_bind(&tenant).unwrap(),
            Value::Text("acme".to_owned())
        );
        assert_eq!(tenant_bind(&Scope::Global).unwrap(), Value::Null);
        // (A future `#[non_exhaustive]` variant would hit the `_` arm and error;
        // there is no way to construct one here, which is the point.)
    }

    #[test]
    fn migrations_name_the_table_and_index() {
        let ddl = delivery_migrations();
        assert_eq!(ddl.len(), 2);
        assert!(ddl[0].contains("CREATE TABLE IF NOT EXISTS seiva_webhook_deliveries"));
        assert!(ddl[1].contains("CREATE INDEX IF NOT EXISTS seiva_webhook_deliveries_due"));
    }
}
