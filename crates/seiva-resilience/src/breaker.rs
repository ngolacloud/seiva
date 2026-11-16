//! A circuit breaker: the guardrail that stops a Seiva service from hammering a
//! dependency that is already down, and lets it recover without a thundering herd.
//!
//! # The three states
//!
//! ```text
//!              failure_threshold consecutive failures
//!   ┌────────┐ ───────────────────────────────────▶ ┌──────┐
//!   │ Closed │                                       │ Open │
//!   └────────┘ ◀─────────────────────────────────── └──────┘
//!       ▲         success_threshold probe successes      │
//!       │                                                │ open_cooldown elapsed
//!       │              any probe fails                   ▼ (one probe admitted)
//!       └──────────────────────────────────────── ┌──────────┐
//!                                                  │ HalfOpen │
//!                                                  └──────────┘
//! ```
//!
//! - **Closed** — calls pass straight through. Consecutive failures are counted;
//!   the `failure_threshold`-th consecutive failure trips the breaker to **Open**.
//!   A single success resets the counter.
//! - **Open** — calls are rejected *without running the operation*
//!   ([`BreakerError::Open`]), shedding load off the failing dependency. After
//!   `open_cooldown` the breaker admits one trial call and moves to **HalfOpen**.
//! - **HalfOpen** — a bounded number of trial calls (`half_open_max`) are let
//!   through to probe recovery; everything else is rejected. `success_threshold`
//!   successes close the breaker; **any** failure re-opens it immediately and
//!   restarts the cooldown.
//!
//! The breaker is cheaply cloneable (an `Arc` inner) and `Send + Sync`, so one
//! handle guards a dependency across every task that calls it.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::time::{SystemTimeSource, TimeSource};

/// How a circuit breaker trips and recovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerConfig {
    /// Consecutive failures in [`Closed`](BreakerState::Closed) that trip the
    /// breaker to [`Open`](BreakerState::Open). Must be `>= 1`.
    pub failure_threshold: u32,
    /// How long the breaker stays [`Open`](BreakerState::Open) — rejecting every
    /// call — before it admits one probe and moves to
    /// [`HalfOpen`](BreakerState::HalfOpen).
    pub open_cooldown: Duration,
    /// How many trial calls [`HalfOpen`](BreakerState::HalfOpen) admits
    /// concurrently to probe recovery. The classic breaker admits one; a higher
    /// value probes faster at the cost of more load on a still-shaky dependency.
    /// Must be `>= 1`.
    pub half_open_max: u32,
    /// How many probe successes in [`HalfOpen`](BreakerState::HalfOpen) close the
    /// breaker. Must be `>= 1`.
    pub success_threshold: u32,
}

impl Default for BreakerConfig {
    /// A conservative default: five consecutive failures trip the breaker, it
    /// cools down for thirty seconds, then a single successful probe closes it.
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            open_cooldown: Duration::from_secs(30),
            half_open_max: 1,
            success_threshold: 1,
        }
    }
}

/// A snapshot of which state the breaker is in, for health reporting and metrics.
/// It carries no counters — it is the *observable* state a readiness probe or a
/// dashboard reports, not the internal bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Calls pass through.
    Closed,
    /// Calls are rejected without running; the dependency is being shielded.
    Open,
    /// A bounded number of trial calls are probing recovery.
    HalfOpen,
}

impl BreakerState {
    /// The lowercase token used in a JSON health body or a metric label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "half_open",
        }
    }
}

/// The internal state machine, carrying the counters [`BreakerState`] hides.
#[derive(Debug, Clone, Copy)]
enum Inner {
    Closed { consecutive_failures: u32 },
    Open { opened_at_millis: u64 },
    HalfOpen { in_flight: u32, successes: u32 },
}

impl Inner {
    fn observable(&self) -> BreakerState {
        match self {
            Inner::Closed { .. } => BreakerState::Closed,
            Inner::Open { .. } => BreakerState::Open,
            Inner::HalfOpen { .. } => BreakerState::HalfOpen,
        }
    }
}

/// The outcome of a guarded call.
#[derive(Debug)]
pub enum BreakerError<E> {
    /// The breaker was open (or half-open at its probe limit): the operation was
    /// **not run**. The caller should fail fast or fall back, not retry into the
    /// open breaker.
    Open,
    /// The operation ran and failed with its own error.
    Inner(E),
}

impl<E> BreakerError<E> {
    /// Whether this is the breaker rejecting the call (vs. the operation failing).
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, BreakerError::Open)
    }
}

impl<E: std::fmt::Display> std::fmt::Display for BreakerError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakerError::Open => f.write_str("circuit breaker is open: call rejected"),
            BreakerError::Inner(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for BreakerError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BreakerError::Open => None,
            BreakerError::Inner(e) => Some(e),
        }
    }
}

struct Shared {
    config: BreakerConfig,
    state: Mutex<Inner>,
    clock: Arc<dyn TimeSource>,
}

/// The decision [`admit`](CircuitBreaker::admit) reaches under the lock: either a
/// call is permitted, or it is rejected because the breaker is open.
enum Admission {
    Permit,
    Reject,
}

/// A circuit breaker guarding one dependency. Clone it freely — every clone shares
/// the same state.
#[derive(Clone)]
pub struct CircuitBreaker {
    shared: Arc<Shared>,
}

impl CircuitBreaker {
    /// A breaker reading the production [`SystemTimeSource`].
    #[must_use]
    pub fn new(config: BreakerConfig) -> Self {
        Self::with_clock(config, Arc::new(SystemTimeSource::new()))
    }

    /// A breaker reading a caller-supplied clock — the seam a test uses to drive
    /// the cooldown with a [`ManualTimeSource`](crate::time::ManualTimeSource)
    /// instead of a real timer.
    #[must_use]
    pub fn with_clock(config: BreakerConfig, clock: Arc<dyn TimeSource>) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                state: Mutex::new(Inner::Closed {
                    consecutive_failures: 0,
                }),
                clock,
            }),
        }
    }

    /// The breaker's current observable state.
    #[must_use]
    pub fn state(&self) -> BreakerState {
        self.locked().observable()
    }

    /// Whether the breaker is currently rejecting calls.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state() == BreakerState::Open
    }

    /// Run `op` under the breaker.
    ///
    /// If the breaker admits the call, `op` runs and its result updates the
    /// breaker (a success or failure is recorded). If the breaker is open — or
    /// half-open and already at its probe limit — `op` is **not run** and the
    /// call returns [`BreakerError::Open`] immediately.
    pub async fn call<F, T, E>(&self, op: F) -> Result<T, BreakerError<E>>
    where
        F: Future<Output = Result<T, E>>,
    {
        match self.admit() {
            Admission::Reject => Err(BreakerError::Open),
            Admission::Permit => match op.await {
                Ok(value) => {
                    self.on_success();
                    Ok(value)
                }
                Err(error) => {
                    self.on_failure();
                    Err(BreakerError::Inner(error))
                }
            },
        }
    }

    /// Decide, under the lock, whether a call is permitted — and perform the
    /// Open→HalfOpen transition when the cooldown has elapsed, so the first caller
    /// after the cooldown becomes the probe.
    fn admit(&self) -> Admission {
        let mut state = self.lock();
        match *state {
            Inner::Closed { .. } => Admission::Permit,
            Inner::Open { opened_at_millis } => {
                let elapsed = self
                    .shared
                    .clock
                    .now_millis()
                    .saturating_sub(opened_at_millis);
                if elapsed >= self.shared.config.open_cooldown.as_millis() as u64 {
                    // Cooldown spent: admit this caller as the first probe.
                    *state = Inner::HalfOpen {
                        in_flight: 1,
                        successes: 0,
                    };
                    tracing::info!(state = "half_open", "circuit breaker probing recovery");
                    Admission::Permit
                } else {
                    Admission::Reject
                }
            }
            Inner::HalfOpen {
                in_flight,
                successes,
            } => {
                if in_flight < self.shared.config.half_open_max {
                    *state = Inner::HalfOpen {
                        in_flight: in_flight + 1,
                        successes,
                    };
                    Admission::Permit
                } else {
                    // Probe budget spent; keep shielding the dependency.
                    Admission::Reject
                }
            }
        }
    }

    /// Record a successful call, advancing the state machine toward Closed.
    fn on_success(&self) {
        let mut state = self.lock();
        match *state {
            Inner::Closed { .. } => {
                // A success clears any partial failure streak.
                *state = Inner::Closed {
                    consecutive_failures: 0,
                };
            }
            Inner::HalfOpen {
                in_flight,
                successes,
            } => {
                let successes = successes + 1;
                if successes >= self.shared.config.success_threshold {
                    *state = Inner::Closed {
                        consecutive_failures: 0,
                    };
                    tracing::info!(state = "closed", "circuit breaker recovered");
                } else {
                    *state = Inner::HalfOpen {
                        in_flight: in_flight.saturating_sub(1),
                        successes,
                    };
                }
            }
            // A success arriving while Open (a probe that raced the cooldown
            // transition) is harmless; the breaker already moved on.
            Inner::Open { .. } => {}
        }
    }

    /// Record a failed call, tripping or re-opening the breaker as the state
    /// requires. Stamps the open time from the breaker's clock.
    fn on_failure(&self) {
        let now = self.shared.clock.now_millis();
        let mut state = self.lock();
        match *state {
            Inner::Closed {
                consecutive_failures,
            } => {
                let failures = consecutive_failures + 1;
                if failures >= self.shared.config.failure_threshold {
                    *state = Inner::Open {
                        opened_at_millis: now,
                    };
                    tracing::warn!(
                        state = "open",
                        failures,
                        "circuit breaker tripped; shedding load off the dependency"
                    );
                } else {
                    *state = Inner::Closed {
                        consecutive_failures: failures,
                    };
                }
            }
            // A probe failed: straight back to Open, cooldown restarts from now.
            Inner::HalfOpen { .. } => {
                *state = Inner::Open {
                    opened_at_millis: now,
                };
                tracing::warn!(state = "open", "circuit breaker probe failed; re-opening");
            }
            Inner::Open { .. } => {}
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn locked(&self) -> Inner {
        *self.lock()
    }
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("config", &self.shared.config)
            .field("state", &self.state())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::ManualTimeSource;

    fn breaker(config: BreakerConfig) -> (CircuitBreaker, Arc<ManualTimeSource>) {
        let clock = Arc::new(ManualTimeSource::new());
        let breaker = CircuitBreaker::with_clock(config, clock.clone());
        (breaker, clock)
    }

    async fn ok(b: &CircuitBreaker) -> Result<(), BreakerError<&'static str>> {
        b.call(async { Ok::<(), &'static str>(()) }).await
    }

    async fn fail(b: &CircuitBreaker) -> Result<(), BreakerError<&'static str>> {
        b.call(async { Err::<(), &'static str>("boom") }).await
    }

    #[tokio::test]
    async fn closed_stays_closed_while_under_the_threshold() {
        let (b, _clock) = breaker(BreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });
        assert_eq!(b.state(), BreakerState::Closed);
        assert!(fail(&b).await.is_err());
        assert!(fail(&b).await.is_err());
        // Two failures, threshold three: still closed, calls still run.
        assert_eq!(b.state(), BreakerState::Closed);
        // A success resets the streak.
        assert!(ok(&b).await.is_ok());
        assert!(fail(&b).await.is_err());
        assert!(fail(&b).await.is_err());
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[tokio::test]
    async fn trips_open_on_consecutive_failures_and_rejects_without_running() {
        let (b, _clock) = breaker(BreakerConfig {
            failure_threshold: 3,
            open_cooldown: Duration::from_secs(30),
            ..Default::default()
        });
        for _ in 0..3 {
            assert!(fail(&b).await.is_err());
        }
        assert_eq!(b.state(), BreakerState::Open);

        // While open the operation must NOT run: prove it by calling an op that
        // would panic if executed — the breaker rejects before awaiting it.
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        let result = b
            .call(async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<(), &'static str>(())
            })
            .await;
        assert!(matches!(result, Err(BreakerError::Open)));
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "an open breaker must reject without running the operation"
        );
    }

    #[tokio::test]
    async fn open_moves_to_half_open_after_cooldown_then_closes_on_success() {
        let (b, clock) = breaker(BreakerConfig {
            failure_threshold: 2,
            open_cooldown: Duration::from_secs(30),
            half_open_max: 1,
            success_threshold: 1,
        });
        // Trip it.
        fail(&b).await.ok();
        fail(&b).await.ok();
        assert_eq!(b.state(), BreakerState::Open);

        // Before the cooldown: still rejected.
        clock.advance_millis(29_000);
        assert!(matches!(ok(&b).await, Err(BreakerError::Open)));
        assert_eq!(b.state(), BreakerState::Open);

        // After the cooldown: the next call is admitted as a probe; it succeeds,
        // which (success_threshold 1) closes the breaker.
        clock.advance_millis(1_000);
        assert!(ok(&b).await.is_ok());
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[tokio::test]
    async fn a_failing_probe_re_opens_the_breaker_and_restarts_the_cooldown() {
        let (b, clock) = breaker(BreakerConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(10),
            half_open_max: 1,
            success_threshold: 1,
        });
        fail(&b).await.ok();
        assert_eq!(b.state(), BreakerState::Open);

        // Cooldown elapses → the next call probes (half-open) and fails → re-open.
        clock.advance_millis(10_000);
        assert!(matches!(fail(&b).await, Err(BreakerError::Inner("boom"))));
        assert_eq!(b.state(), BreakerState::Open);

        // The cooldown restarted from the probe failure: an immediate call is
        // still rejected, proving the clock was re-stamped.
        assert!(matches!(ok(&b).await, Err(BreakerError::Open)));
    }

    #[tokio::test]
    async fn half_open_admits_at_most_half_open_max_probes() {
        let (b, clock) = breaker(BreakerConfig {
            failure_threshold: 1,
            open_cooldown: Duration::from_secs(5),
            half_open_max: 1,
            success_threshold: 2,
        });
        fail(&b).await.ok();
        clock.advance_millis(5_000);

        // Hold the first probe open so it stays in-flight while a second call
        // tries to enter half-open.
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate2 = gate.clone();
        let b2 = b.clone();
        let probe = tokio::spawn(async move {
            b2.call(async move {
                gate2.notified().await;
                Ok::<(), &'static str>(())
            })
            .await
        });
        // Give the probe a moment to be admitted and park on the gate.
        tokio::task::yield_now().await;
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert_eq!(b.state(), BreakerState::HalfOpen);
        // A second call while the probe is in-flight: budget is one, so rejected.
        assert!(matches!(ok(&b).await, Err(BreakerError::Open)));

        // Release the first probe; one success is not enough (threshold two), so
        // the breaker is still half-open and now admits the next probe.
        gate.notify_one();
        assert!(probe.await.unwrap().is_ok());
        assert_eq!(b.state(), BreakerState::HalfOpen);
        assert!(ok(&b).await.is_ok());
        assert_eq!(b.state(), BreakerState::Closed);
    }
}
