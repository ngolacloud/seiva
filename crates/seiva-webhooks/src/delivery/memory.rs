//! An in-memory [`DeliveryLedger`] — the dev/test double.
//!
//! It keeps the same guarantees the engine relies on, in a `HashMap` under a
//! mutex: idempotent enqueue, a due-and-leased `claim_due` (with orphan recovery
//! of an expired lease), and fail-closed on a missing tenant scope. A product
//! swaps in a DORM-backed row for real durability; nothing in the engine changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::ledger::{
    Delivery, DeliveryId, DeliveryLedger, DeliveryStatus, DeliveryUpdate, LedgerError,
    LedgerResult, NewDelivery,
};

/// A stored row: the delivery plus the lease that leases it to one worker.
#[derive(Debug, Clone)]
struct Row {
    delivery: Delivery,
    /// When the current `InFlight` lease expires. Meaningless unless the row is
    /// `InFlight`; an expired lease makes an orphaned row claimable again.
    lease_until: SystemTime,
}

/// An in-memory delivery ledger. Cheap to clone (an `Arc` inside), so a test
/// holds one handle while the engine holds another over the **same** rows.
#[derive(Clone, Default)]
pub struct InMemoryLedger {
    rows: Arc<Mutex<HashMap<String, Row>>>,
}

impl InMemoryLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many rows the ledger holds, for test assertions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether the ledger holds no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Row>> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl DeliveryLedger for InMemoryLedger {
    async fn enqueue(&self, new: NewDelivery) -> LedgerResult<()> {
        // Fail closed on a missing tenant scope (ADR 0019): a tenant-less
        // delivery is a bug, not a shared-root send.
        let scope = new.scope.ok_or(LedgerError::MissingScope)?;
        let mut rows = self.lock();
        // Idempotent on id: a replayed outbox drain never duplicates a delivery.
        if rows.contains_key(new.id.as_str()) {
            return Ok(());
        }
        let delivery = Delivery {
            id: new.id.clone(),
            subscription: new.subscription,
            event_id: new.event_id,
            endpoint: new.endpoint,
            payload: new.payload,
            scope,
            status: DeliveryStatus::Pending,
            attempt: 0,
            // Due immediately: the epoch is `<= now` for any real clock, so the
            // next tick claims it without the ledger needing to read a clock.
            next_attempt_at: UNIX_EPOCH,
            last_http_status: None,
            last_response_excerpt: None,
        };
        rows.insert(
            new.id.as_str().to_string(),
            Row {
                delivery,
                lease_until: UNIX_EPOCH,
            },
        );
        Ok(())
    }

    async fn claim_due(
        &self,
        now: SystemTime,
        max: usize,
        lease: Duration,
    ) -> LedgerResult<Vec<Delivery>> {
        let mut rows = self.lock();
        // A row is due if it is pending-and-arrived, or in-flight with a lapsed
        // lease (an orphan from a crashed worker).
        let mut candidates: Vec<String> = rows
            .values()
            .filter(|row| match row.delivery.status {
                DeliveryStatus::Pending => row.delivery.next_attempt_at <= now,
                DeliveryStatus::InFlight => row.lease_until <= now,
                _ => false,
            })
            .map(|row| row.delivery.id.as_str().to_string())
            .collect();
        // Stable, fair order (earliest due first, then id) so claiming is
        // deterministic regardless of the map's iteration order.
        candidates.sort_by(|a, b| {
            let ra = &rows[a].delivery.next_attempt_at;
            let rb = &rows[b].delivery.next_attempt_at;
            ra.cmp(rb).then_with(|| a.cmp(b))
        });
        candidates.truncate(max);

        let lease_until = now + lease;
        let mut claimed = Vec::with_capacity(candidates.len());
        for id in candidates {
            let row = rows.get_mut(&id).expect("candidate id came from the map");
            row.delivery.status = DeliveryStatus::InFlight;
            row.lease_until = lease_until;
            claimed.push(row.delivery.clone());
        }
        Ok(claimed)
    }

    async fn record(&self, id: &DeliveryId, update: DeliveryUpdate) -> LedgerResult<()> {
        let mut rows = self.lock();
        let row = rows.get_mut(id.as_str()).ok_or(LedgerError::NotFound)?;
        row.delivery.status = update.status;
        row.delivery.attempt = update.attempt;
        row.delivery.next_attempt_at = update.next_attempt_at;
        row.delivery.last_http_status = update.last_http_status;
        row.delivery.last_response_excerpt = update.last_response_excerpt;
        // A terminal row holds no lease; a rescheduled one is claimable at its
        // next_attempt_at, so its lease is irrelevant until then.
        row.lease_until = update.next_attempt_at;
        Ok(())
    }

    async fn get(&self, id: &DeliveryId) -> LedgerResult<Option<Delivery>> {
        Ok(self.lock().get(id.as_str()).map(|row| row.delivery.clone()))
    }
}
