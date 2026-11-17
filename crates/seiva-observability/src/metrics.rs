//! Prometheus metrics: a recorder for the [`metrics`] facade and a `/metrics`
//! scrape surface served on a dedicated bind.
//!
//! The library crates record against the `metrics` facade (counters, histograms,
//! gauges) unconditionally — those calls are a no-op until [`install_recorder`]
//! installs the global Prometheus recorder here. [`serve_metrics`] then exposes
//! the snapshot on a **separate** bind (an admin port), kept off the public API
//! surface and exempt from auth/rate-limiting like the health probes (ADR 0012).

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder, PrometheusHandle};
use tokio::net::TcpListener;

/// Latency histogram buckets (seconds) for the HTTP request-duration metric:
/// sub-millisecond to ~16 s, so both fast reads and slow writes land in a bucket.
const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 16.0,
];

/// Installs the process-global Prometheus recorder backing the [`metrics`] facade
/// and returns a handle that renders the current snapshot in the Prometheus text
/// exposition format.
///
/// The `metrics` facade allows exactly **one** global recorder per process, so
/// this is a boot-time, call-once operation: a second call returns a
/// [`BuildError`]. A service installs it once (the `seiva-server` root does this
/// behind `with_metrics`).
///
/// # Errors
///
/// Returns [`BuildError`] if the bucket configuration is rejected or a recorder is
/// already installed.
pub fn install_recorder() -> Result<PrometheusHandle, BuildError> {
    PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS)?
        .install_recorder()
}

/// Builds the `/metrics` router that renders `handle`'s snapshot on each scrape.
/// Mounted alone on its own bind by [`serve_metrics`]; it carries no auth, rate
/// limit or module route — only the exposition endpoint a Prometheus server polls.
pub fn metrics_router(handle: PrometheusHandle) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let handle = handle.clone();
            async move { handle.render() }
        }),
    )
}

/// Serves `/metrics` on `addr` until `shutdown` resolves, then returns.
///
/// A **separate** bind from the public API: the scrape surface is an admin port,
/// not the service's data plane, so the platform scrapes it on the pod network
/// without exposing it to API callers. The `seiva-server` root drives this next
/// to the main server, sharing one shutdown signal.
///
/// # Errors
///
/// Returns the [`std::io::Error`] from binding the listener or from serving.
pub async fn serve_metrics(
    addr: SocketAddr,
    handle: PrometheusHandle,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, metrics_router(handle))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_buckets_are_sorted_and_positive() {
        assert!(LATENCY_BUCKETS.windows(2).all(|w| w[0] < w[1]));
        assert!(LATENCY_BUCKETS.iter().all(|&b| b > 0.0));
    }
}
