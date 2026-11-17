//! A retry policy with exponential backoff — the same schedule the outbox relay
//! paces its attempts by, lifted here as a reusable primitive.
//!
//! The backoff formula is **byte-for-byte** `seiva_events::RelayPolicy::backoff`
//! (and therefore `dorm_outbox::relay`): the base doubles each attempt, capped at
//! 30× the base. A transient failure is thus paced identically whether the
//! transactional-outbox relay retries a broker publish or a caller wraps a plain
//! operation here, so there is one retry schedule across the framework, not two
//! drifting sets of constants.

use std::future::Future;
use std::time::Duration;

/// How many times an operation is retried, and how long to wait between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The total number of attempts, including the first. `1` means "try once, do
    /// not retry". Must be `>= 1`.
    pub max_attempts: u32,
    /// The base backoff between attempts, in milliseconds. It doubles each
    /// attempt, capped at 30× the base — the exact schedule
    /// `dorm_outbox::relay` and `seiva_events::RelayPolicy` sleep.
    pub backoff_base_ms: u64,
}

impl Default for RetryPolicy {
    /// Five attempts with a 50 ms base backoff — the same failure policy as
    /// `seiva_events::RelayPolicy::default`, so a caller that does not override it
    /// gets the framework-wide schedule.
    fn default() -> Self {
        Self {
            max_attempts: 5,
            backoff_base_ms: 50,
        }
    }
}

impl RetryPolicy {
    /// The backoff before the `attempt`-th retry (1-based), doubling each attempt
    /// and capped at 30× the base — byte-for-byte `RelayPolicy::backoff`, so a
    /// transient failure is paced identically here and on the outbox relay path.
    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        let mult = 1u64 << attempt.min(5); // 1, 2, 4, …, 32
        Duration::from_millis(
            self.backoff_base_ms
                .saturating_mul(mult)
                .min(self.backoff_base_ms.saturating_mul(30)),
        )
    }

    /// Run `op`, retrying every failure up to [`max_attempts`](Self::max_attempts)
    /// times with [`backoff`](Self::backoff) between attempts. Returns the first
    /// `Ok`, or the last `Err` once the budget is spent.
    ///
    /// `op` is a closure returning a fresh future per attempt, so the operation is
    /// actually re-run (not a single future polled twice).
    pub async fn retry<F, Fut, T, E>(&self, op: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.retry_if(op, |_| true).await
    }

    /// Like [`retry`](Self::retry), but only retries errors for which
    /// `is_retryable` returns `true`. A permanent error (a `400`, a validation
    /// failure) short-circuits immediately instead of burning the whole attempt
    /// budget on something that will never succeed.
    pub async fn retry_if<F, Fut, T, E, P>(&self, mut op: F, mut is_retryable: P) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        P: FnMut(&E) -> bool,
    {
        // At least one attempt, whatever the config says.
        let attempts = self.max_attempts.max(1);
        let mut attempt = 1;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if attempt >= attempts || !is_retryable(&error) {
                        return Err(error);
                    }
                    let wait = self.backoff(attempt);
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                    attempt += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    // A zero-base policy so the retries do not actually sleep in the test.
    fn instant(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            backoff_base_ms: 0,
        }
    }

    #[test]
    fn backoff_matches_the_relay_schedule() {
        // The RelayPolicy schedule: base doubles, capped at 30× base.
        let p = RetryPolicy {
            max_attempts: 10,
            backoff_base_ms: 50,
        };
        assert_eq!(p.backoff(0), Duration::from_millis(50)); // 1×
        assert_eq!(p.backoff(1), Duration::from_millis(100)); // 2×
        assert_eq!(p.backoff(2), Duration::from_millis(200)); // 4×
        assert_eq!(p.backoff(3), Duration::from_millis(400)); // 8×
        assert_eq!(p.backoff(4), Duration::from_millis(800)); // 16×
                                                              // 32× would be 1600, capped at 30× = 1500.
        assert_eq!(p.backoff(5), Duration::from_millis(1500));
        assert_eq!(p.backoff(9), Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn retries_until_success_then_stops() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let p = instant(5);
        let result: Result<u32, &str> = p
            .retry(|| {
                let c = c.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                    if n >= 3 {
                        Ok(n)
                    } else {
                        Err("transient")
                    }
                }
            })
            .await;
        assert_eq!(result, Ok(3));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "stops retrying once it succeeds"
        );
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts_returning_the_last_error() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let p = instant(3);
        let result: Result<(), &str> = p
            .retry(|| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("always")
                }
            })
            .await;
        assert_eq!(result, Err("always"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "exactly max_attempts tries"
        );
    }

    #[tokio::test]
    async fn a_non_retryable_error_short_circuits() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let p = instant(5);
        let result: Result<(), &str> = p
            .retry_if(
                || {
                    let c = c.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Err("permanent")
                    }
                },
                |e| *e != "permanent",
            )
            .await;
        assert_eq!(result, Err("permanent"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a non-retryable error is not retried"
        );
    }

    #[tokio::test]
    async fn max_attempts_zero_still_runs_once() {
        let p = instant(0);
        let result: Result<u8, &str> = p.retry(|| async { Ok(7) }).await;
        assert_eq!(result, Ok(7));
    }
}
