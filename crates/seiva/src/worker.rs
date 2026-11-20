//! The worker runtime: drive a one-pass job in a loop, with an interval,
//! graceful shutdown, and metrics.
//!
//! This is the framework half of the boundary the brief draws (Parte B / the ORM
//! boundary): the **one-pass** functions belong to the data layer — the outbox
//! relay's `forward`/`drain`, RAG's `run_once`, a `dorm-jobs` lease pass — each
//! does one pass and returns how many items it handled; the **loop** that repeats
//! them, with timing, shutdown and metrics, is the framework's. [`run_worker`]
//! is that loop, generic over any such pass, so it is data-layer-free and
//! testable with a plain closure.

use std::future::Future;
use std::time::Duration;

use seiva_ports::PortResult;

/// How a worker loops.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// A label for logs and metrics.
    pub name: String,
    /// How long to wait between passes. `Duration::ZERO` runs the next pass as
    /// soon as the last returns.
    pub interval: Duration,
    /// When a pass processed at least this many items, run the next immediately
    /// (drain fast while there is a backlog) instead of waiting `interval`. `0`
    /// disables the fast path.
    pub drain_threshold: u64,
}

impl WorkerConfig {
    /// A worker named `name` that waits `interval` between passes, with the
    /// fast-drain path off.
    pub fn new(name: impl Into<String>, interval: Duration) -> Self {
        Self {
            name: name.into(),
            interval,
            drain_threshold: 0,
        }
    }

    /// Enable the fast-drain path: a pass that processed `>= threshold` items is
    /// followed immediately by the next, without waiting `interval`.
    pub fn drain_fast_at(mut self, threshold: u64) -> Self {
        self.drain_threshold = threshold;
        self
    }
}

/// What a worker did between start and shutdown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerReport {
    /// How many passes ran.
    pub passes: u64,
    /// Total items processed across all passes.
    pub processed: u64,
    /// How many passes returned an error (logged, never fatal).
    pub errors: u64,
}

/// Run `pass` in a loop until `shutdown` resolves, then stop (drain-and-exit).
///
/// Each pass returns how many items it processed. An `Err` from a pass is logged
/// and counted but **never fatal** — a worker outlives a transient failure.
/// Between passes the loop waits `config.interval`, unless the last pass hit
/// `config.drain_threshold`, in which case it runs again immediately. Shutdown is
/// honoured promptly: it is checked before each wait, so a resolved `shutdown`
/// ends the loop within one pass. Returns the accumulated [`WorkerReport`].
pub async fn run_worker<F, Fut, S>(config: WorkerConfig, shutdown: S, mut pass: F) -> WorkerReport
where
    F: FnMut() -> Fut,
    Fut: Future<Output = PortResult<u64>>,
    S: Future<Output = ()>,
{
    let mut report = WorkerReport::default();
    let mut shutdown = std::pin::pin!(shutdown);

    loop {
        report.passes += 1;
        let processed = match pass().await {
            Ok(n) => {
                report.processed += n;
                n
            }
            Err(error) => {
                report.errors += 1;
                tracing::warn!(
                    target: "seiva::worker",
                    worker = %config.name,
                    error = %error,
                    "worker pass failed; continuing",
                );
                0
            }
        };

        let wait = if config.drain_threshold > 0 && processed >= config.drain_threshold {
            Duration::ZERO
        } else {
            config.interval
        };

        // Wait the interval, but stop at once if shutdown has fired (biased: it
        // wins a tie, so a ready shutdown always ends the loop here).
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(wait) => {}
        }
    }

    tracing::info!(
        target: "seiva::worker",
        worker = %config.name,
        passes = report.passes,
        processed = report.processed,
        errors = report.errors,
        "worker stopped",
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    fn immediate() -> WorkerConfig {
        WorkerConfig::new("test", Duration::ZERO)
    }

    #[tokio::test]
    async fn runs_passes_until_shutdown_and_accumulates_metrics() {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let calls = AtomicU64::new(0);

        let report = run_worker(
            immediate(),
            async {
                let _ = rx.await;
            },
            || {
                // The third pass signals shutdown; the loop then ends after it.
                if calls.fetch_add(1, Ordering::SeqCst) + 1 == 3 {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }
                async { Ok(10) }
            },
        )
        .await;

        assert_eq!(report.passes, 3);
        assert_eq!(report.processed, 30);
        assert_eq!(report.errors, 0);
    }

    #[tokio::test]
    async fn an_erroring_pass_is_counted_and_not_fatal() {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let calls = AtomicU64::new(0);

        let report = run_worker(
            immediate(),
            async {
                let _ = rx.await;
            },
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 2 {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }
                async move {
                    if n == 1 {
                        Err("transient".into()) // first pass fails, worker continues
                    } else {
                        Ok(5)
                    }
                }
            },
        )
        .await;

        assert_eq!(report.passes, 2);
        assert_eq!(report.processed, 5);
        assert_eq!(report.errors, 1);
    }
}
