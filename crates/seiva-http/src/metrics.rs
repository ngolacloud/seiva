//! Per-request metrics, recorded against the [`metrics`] facade (ADR 0027).
//!
//! [`track_metrics`] is an axum middleware that records, for every request that
//! reaches a route: a request **counter**, a latency **histogram** and an
//! in-flight **gauge**. The series are **low-cardinality by construction** — the
//! labels are the matched **route template** (never the concrete path, so an id
//! or a 404-scanner cannot explode the series space), the HTTP **method** and the
//! response **status class** (`2xx`/`4xx`/`5xx`). The tenant is deliberately
//! **not** a label (it belongs on a span, not in the Prometheus series space).
//!
//! The recording is a no-op until a recorder is installed at the root
//! (`seiva-observability`'s `install_recorder`), so this crate stays DORM-free and
//! costs only an `Instant::now` and a facade call when no exporter is wired.
//!
//! Health probes are **exempt** (ADR 0012): `/health/*` carries no metric noise,
//! exactly as it is exempt from the rate limiter.

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use metrics::Label;

/// The request counter: total requests, by `method`, `route` and `status` class.
pub const REQUESTS_TOTAL: &str = "seiva_http_requests_total";
/// The request-latency histogram (seconds), by `method`, `route` and `status`.
pub const REQUEST_DURATION_SECONDS: &str = "seiva_http_request_duration_seconds";
/// The in-flight gauge: requests currently being handled (unlabelled).
pub const REQUESTS_IN_FLIGHT: &str = "seiva_http_requests_in_flight";

/// The route label used when no route matched (a 404). A single constant keeps a
/// path-scanner from minting one series per bogus path — cardinality stays bounded.
const UNMATCHED_ROUTE: &str = "<unmatched>";

/// Collapses a status code into its class, so the `status` label has at most five
/// values instead of one per code.
fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

/// Decrements the in-flight gauge on drop, so a cancelled or panicking request
/// (client disconnect, timeout, handler panic) still leaves the gauge balanced.
struct InFlightGuard;

impl InFlightGuard {
    fn enter() -> Self {
        metrics::gauge!(REQUESTS_IN_FLIGHT).increment(1.0);
        Self
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        metrics::gauge!(REQUESTS_IN_FLIGHT).decrement(1.0);
    }
}

/// Middleware recording the request counter, latency histogram and in-flight gauge
/// against the [`metrics`] facade. Mounted in the framework layer stack; see the
/// module docs for the cardinality guarantees. Health probes are skipped.
pub async fn track_metrics(request: Request, next: Next) -> Response {
    // Health probes are exempt from instrumentation noise (ADR 0012), like the
    // rate limiter. Check the raw path before any recording.
    if request.uri().path().starts_with("/health") {
        return next.run(request).await;
    }

    // The matched route TEMPLATE (e.g. `/v1/projects/{id}`), never the concrete
    // path: this is the single most important cardinality control. Absent on a
    // 404 — collapsed to one `<unmatched>` series.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| UNMATCHED_ROUTE.to_owned());
    let method = request.method().as_str().to_owned();

    let guard = InFlightGuard::enter();
    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed().as_secs_f64();
    drop(guard);

    let labels = vec![
        Label::new("method", method),
        Label::new("route", route),
        Label::new("status", status_class(response.status().as_u16())),
    ];
    metrics::counter!(REQUESTS_TOTAL, labels.clone()).increment(1);
    metrics::histogram!(REQUEST_DURATION_SECONDS, labels).record(elapsed);

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes_collapse_to_five_buckets() {
        assert_eq!(status_class(100), "1xx");
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(301), "3xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(599), "5xx");
    }
}
