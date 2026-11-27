//! Budgets: tokens per principal, per tenant and per area, per period.
//!
//! The gateway reserves an estimate before the call (the request's size plus its
//! `max_tokens`) and settles the real usage after it. A store that cannot answer
//! stops the call — unlike the HTTP rate limiter (ADR 0015), which fails open: an
//! unmetered model call is spend nobody approved.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use seiva_ports::Clock;

/// What a budget is kept for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BudgetScope {
    /// One caller.
    Principal,
    /// One tenant.
    Tenant,
    /// One area (a department).
    Area,
}

/// One budget: a scope and an id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BudgetKey {
    /// The scope.
    pub scope: BudgetScope,
    /// The principal's subject, the tenant or the area.
    pub id: String,
}

impl BudgetKey {
    /// A key.
    pub fn new(scope: BudgetScope, id: impl Into<String>) -> Self {
        Self {
            scope,
            id: id.into(),
        }
    }
}

impl fmt::Display for BudgetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scope = match self.scope {
            BudgetScope::Principal => "principal",
            BudgetScope::Tenant => "tenant",
            BudgetScope::Area => "area",
        };
        write!(f, "{scope}:{}", self.id)
    }
}

/// Units held for one call until it settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    /// The store's id for it.
    pub id: u64,
    /// The budgets it holds units in.
    pub keys: Vec<BudgetKey>,
    /// The units held.
    pub units: u64,
}

/// Why a reservation was not granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// This budget would go over its limit.
    Exceeded(BudgetKey),
    /// The store could not answer.
    Unavailable(String),
}

/// Where budgets are kept. Reserving must be atomic across all the keys: either
/// every budget holds the units, or none does.
#[async_trait]
pub trait BudgetStore: Send + Sync {
    /// Hold `units` in every budget of `keys` that has a limit.
    async fn reserve(&self, keys: &[BudgetKey], units: u64) -> Result<Reservation, BudgetError>;

    /// Release the reservation and count `actual` units as used.
    async fn settle(&self, reservation: &Reservation, actual: u64) -> Result<(), BudgetError>;
}

#[derive(Debug, Default, Clone, Copy)]
struct Window {
    index: u64,
    used: u64,
    reserved: u64,
}

/// An in-process [`BudgetStore`] with fixed windows of `period`. Limits are set
/// per scope (every principal gets N) and per key (this tenant gets M); a key
/// with neither has no limit. One process only — a multi-replica deployment needs
/// a shared store (a Redis one following ADR 0015's atomic pattern).
pub struct InMemoryBudget {
    clock: Arc<dyn Clock>,
    period: Duration,
    per_scope: HashMap<BudgetScope, u64>,
    per_key: HashMap<BudgetKey, u64>,
    windows: Mutex<HashMap<BudgetKey, Window>>,
    next_id: AtomicU64,
}

impl InMemoryBudget {
    /// Windows of `period`, read off `clock`. No limits yet.
    pub fn new(clock: Arc<dyn Clock>, period: Duration) -> Self {
        Self {
            clock,
            period: period.max(Duration::from_secs(1)),
            per_scope: HashMap::new(),
            per_key: HashMap::new(),
            windows: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Every key of `scope` gets `units` per period.
    #[must_use]
    pub fn per(mut self, scope: BudgetScope, units: u64) -> Self {
        self.per_scope.insert(scope, units);
        self
    }

    /// This key gets `units` per period (over its scope's default).
    #[must_use]
    pub fn limit(mut self, key: BudgetKey, units: u64) -> Self {
        self.per_key.insert(key, units);
        self
    }

    fn limit_of(&self, key: &BudgetKey) -> Option<u64> {
        self.per_key
            .get(key)
            .or_else(|| self.per_scope.get(&key.scope))
            .copied()
    }

    fn window_index(&self) -> u64 {
        let secs = self
            .clock
            .now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        secs / self.period.as_secs()
    }

    /// Units used in the current window by `key`.
    pub fn used(&self, key: &BudgetKey) -> u64 {
        let index = self.window_index();
        let windows = self.windows.lock().expect("budget poisoned");
        windows
            .get(key)
            .filter(|w| w.index == index)
            .map_or(0, |w| w.used)
    }
}

#[async_trait]
impl BudgetStore for InMemoryBudget {
    async fn reserve(&self, keys: &[BudgetKey], units: u64) -> Result<Reservation, BudgetError> {
        let index = self.window_index();
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| BudgetError::Unavailable("budget state poisoned".into()))?;
        let limited: Vec<(BudgetKey, u64)> = keys
            .iter()
            .filter_map(|k| self.limit_of(k).map(|l| (k.clone(), l)))
            .collect();
        for (key, limit) in &limited {
            let w = windows
                .get(key)
                .copied()
                .filter(|w| w.index == index)
                .unwrap_or_default();
            if w.used.saturating_add(w.reserved).saturating_add(units) > *limit {
                return Err(BudgetError::Exceeded(key.clone()));
            }
        }
        for (key, _) in &limited {
            let w = windows.entry(key.clone()).or_default();
            if w.index != index {
                *w = Window {
                    index,
                    ..Window::default()
                };
            }
            w.reserved += units;
        }
        Ok(Reservation {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            keys: limited.into_iter().map(|(k, _)| k).collect(),
            units,
        })
    }

    async fn settle(&self, reservation: &Reservation, actual: u64) -> Result<(), BudgetError> {
        let index = self.window_index();
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| BudgetError::Unavailable("budget state poisoned".into()))?;
        for key in &reservation.keys {
            let w = windows.entry(key.clone()).or_default();
            if w.index != index {
                // The window rolled over while the call ran: the old hold is gone.
                *w = Window {
                    index,
                    ..Window::default()
                };
            } else {
                w.reserved = w.reserved.saturating_sub(reservation.units);
            }
            w.used = w.used.saturating_add(actual);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_testing::FixedClock;

    fn store(clock: &FixedClock) -> InMemoryBudget {
        InMemoryBudget::new(Arc::new(clock.clone()), Duration::from_secs(3600))
            .per(BudgetScope::Principal, 1000)
            .limit(BudgetKey::new(BudgetScope::Tenant, "acme"), 1500)
    }

    #[tokio::test]
    async fn a_reservation_is_refused_when_it_would_pass_a_limit() {
        let clock = FixedClock::at_unix_secs(7200);
        let s = store(&clock);
        let ana = BudgetKey::new(BudgetScope::Principal, "ana");
        let acme = BudgetKey::new(BudgetScope::Tenant, "acme");
        let r = s.reserve(&[ana.clone(), acme.clone()], 600).await.unwrap();
        // Held units count: 600 + 600 > 1000 for ana.
        assert_eq!(
            s.reserve(std::slice::from_ref(&ana), 600).await,
            Err(BudgetError::Exceeded(ana.clone()))
        );
        s.settle(&r, 300).await.unwrap();
        assert_eq!(s.used(&ana), 300);
        // 300 used by ana (and by acme): 600 more fits for ana…
        assert!(s.reserve(std::slice::from_ref(&ana), 600).await.is_ok());
        // …but 1300 more does not fit for acme (300 + 1300 > 1500).
        assert_eq!(
            s.reserve(std::slice::from_ref(&acme), 1300).await,
            Err(BudgetError::Exceeded(acme))
        );
    }

    #[tokio::test]
    async fn a_refused_reservation_holds_nothing_anywhere() {
        let clock = FixedClock::at_unix_secs(0);
        let s = store(&clock);
        let ana = BudgetKey::new(BudgetScope::Principal, "ana");
        let acme = BudgetKey::new(BudgetScope::Tenant, "acme");
        assert!(s.reserve(&[ana.clone(), acme.clone()], 1200).await.is_err());
        // Nothing was held for acme by the refused call.
        assert!(s.reserve(&[acme], 1500).await.is_ok());
    }

    #[tokio::test]
    async fn the_window_resets_after_the_period_and_unlimited_keys_are_not_tracked() {
        let clock = FixedClock::at_unix_secs(0);
        let s = store(&clock);
        let ana = BudgetKey::new(BudgetScope::Principal, "ana");
        let r = s.reserve(std::slice::from_ref(&ana), 900).await.unwrap();
        s.settle(&r, 900).await.unwrap();
        assert!(s.reserve(std::slice::from_ref(&ana), 200).await.is_err());
        clock.advance(Duration::from_secs(3600));
        assert!(s.reserve(std::slice::from_ref(&ana), 200).await.is_ok());
        let area = BudgetKey::new(BudgetScope::Area, "hr");
        let r = s.reserve(&[area], 1_000_000).await.unwrap();
        assert!(r.keys.is_empty());
    }
}
