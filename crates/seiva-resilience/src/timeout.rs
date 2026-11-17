//! A timeout wrapper: cut an operation off with a clean, typed error if it
//! outlives its budget, so a slow dependency cannot pin a request handler or a
//! worker pass forever.
//!
//! This is a thin, typed shell over [`tokio::time::timeout`]: it maps the
//! runtime's opaque elapsed marker to a [`TimedOut`] that names the budget it
//! blew, so a caller can `?`-propagate it as a real `std::error::Error`.

use std::future::Future;
use std::time::Duration;

/// The error returned when an operation outlives its timeout budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation timed out after {budget:?}")]
pub struct TimedOut {
    /// The budget the operation blew through.
    pub budget: Duration,
}

/// Run `fut`, returning its output if it completes within `budget`, or
/// [`TimedOut`] if it does not. The future is dropped (cancelled) on timeout.
///
/// ```no_run
/// # use std::time::Duration;
/// # use seiva_resilience::timeout;
/// # async fn demo() {
/// let result = timeout(Duration::from_millis(10), async { 42 }).await;
/// assert!(result.is_ok());
/// # }
/// # let _ = demo;
/// ```
pub async fn timeout<F>(budget: Duration, fut: F) -> Result<F::Output, TimedOut>
where
    F: Future,
{
    tokio::time::timeout(budget, fut)
        .await
        .map_err(|_| TimedOut { budget })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_fast_operation_returns_its_value() {
        let result = timeout(Duration::from_secs(1), async { 7u8 }).await;
        assert_eq!(result, Ok(7));
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_operation_times_out_naming_its_budget() {
        let budget = Duration::from_millis(50);
        let result = timeout(budget, async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            7u8
        })
        .await;
        assert_eq!(result, Err(TimedOut { budget }));
    }
}
