//! End-to-end proof, through the public API only, that the circuit breaker walks
//! the full `Closed → Open → HalfOpen → Closed` lifecycle and composes with the
//! retry policy. Drives the cooldown with a [`ManualTimeSource`] so the whole
//! lifecycle runs with no real sleeping.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use seiva_resilience::time::ManualTimeSource;
use seiva_resilience::{BreakerConfig, BreakerError, BreakerState, CircuitBreaker, RetryPolicy};

#[tokio::test]
async fn breaker_walks_closed_open_half_open_closed() {
    let clock = Arc::new(ManualTimeSource::new());
    let breaker = CircuitBreaker::with_clock(
        BreakerConfig {
            failure_threshold: 3,
            open_cooldown: Duration::from_secs(20),
            half_open_max: 1,
            success_threshold: 2,
        },
        clock.clone(),
    );

    // A dependency switch the test flips: fails while `false`, succeeds once `true`.
    let healthy = Arc::new(AtomicBool::new(false));

    let hit = |breaker: &CircuitBreaker, healthy: Arc<AtomicBool>| {
        let breaker = breaker.clone();
        async move {
            breaker
                .call(async move {
                    if healthy.load(Ordering::SeqCst) {
                        Ok::<&str, &str>("ok")
                    } else {
                        Err::<&str, &str>("down")
                    }
                })
                .await
        }
    };

    // 1) CLOSED → OPEN: three failures trip the breaker.
    assert_eq!(breaker.state(), BreakerState::Closed);
    for _ in 0..3 {
        assert!(matches!(
            hit(&breaker, healthy.clone()).await,
            Err(BreakerError::Inner("down"))
        ));
    }
    assert_eq!(breaker.state(), BreakerState::Open);

    // 2) OPEN: calls are rejected without running, even though the dependency has
    // actually recovered — the breaker is still shielding it.
    healthy.store(true, Ordering::SeqCst);
    assert!(matches!(
        hit(&breaker, healthy.clone()).await,
        Err(BreakerError::Open)
    ));

    // 3) OPEN → HALF-OPEN → CLOSED: once the cooldown elapses, probes are admitted;
    // success_threshold (2) successful probes close the breaker.
    clock.advance_millis(20_000);
    assert!(hit(&breaker, healthy.clone()).await.is_ok()); // probe 1 (half-open)
    assert_eq!(breaker.state(), BreakerState::HalfOpen);
    assert!(hit(&breaker, healthy.clone()).await.is_ok()); // probe 2 → close
    assert_eq!(breaker.state(), BreakerState::Closed);

    // 4) Back to normal service.
    assert!(hit(&breaker, healthy.clone()).await.is_ok());
    assert_eq!(breaker.state(), BreakerState::Closed);
}

#[tokio::test]
async fn retry_recovers_a_transient_failure_without_tripping_the_breaker() {
    let breaker = CircuitBreaker::new(BreakerConfig {
        failure_threshold: 2,
        ..Default::default()
    });
    // Zero backoff so the test does not sleep.
    let retry = RetryPolicy {
        max_attempts: 4,
        backoff_base_ms: 0,
    };
    let attempts = Arc::new(AtomicU32::new(0));

    // The breaker sees ONE call (the retry loop lives inside it); the retry
    // absorbs the two transient failures, so the breaker never counts a failure.
    let a = attempts.clone();
    let result: Result<u32, BreakerError<&str>> = breaker
        .call(async move {
            retry
                .retry(|| {
                    let a = a.clone();
                    async move {
                        let n = a.fetch_add(1, Ordering::SeqCst) + 1;
                        if n >= 3 {
                            Ok(n)
                        } else {
                            Err("transient")
                        }
                    }
                })
                .await
        })
        .await;

    assert_eq!(result.unwrap(), 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        breaker.state(),
        BreakerState::Closed,
        "a retried-away transient failure must not trip the breaker"
    );
}
