//! Integration proof for ADR 0027 metrics: the framework app records a
//! low-cardinality HTTP series against the `metrics` facade, and the Prometheus
//! recorder (installed by `seiva-observability`) exposes it.
//!
//! Gated on the `metrics` feature, so plain `cargo test --workspace` (default
//! features) skips it; run it with `cargo test -p seiva-server --features metrics`.
//! Driven through the router with `oneshot` — no live port, deterministic.
#![cfg(feature = "metrics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use seiva_http::Health;
use tower::ServiceExt; // for `oneshot`

async fn get(app: &axum::Router, uri: &str) -> StatusCode {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn metrics_records_low_cardinality_http_series() {
    // The Prometheus recorder is process-global and install-once; this test owns
    // it (it is the only test in this binary that installs one).
    let handle =
        seiva_observability::metrics::install_recorder().expect("the recorder installs once");

    // The framework app (health, `/openapi.json`, `/docs`) carries the metrics
    // middleware through `apply_framework_layers`.
    let app = seiva_http::router(Health::starting());

    // A matched route (2xx)...
    assert_eq!(get(&app, "/openapi.json").await, StatusCode::OK);
    // ...a health probe, which must be EXEMPT from metrics (ADR 0012)...
    assert_eq!(get(&app, "/health/live").await, StatusCode::OK);
    // ...and an unmatched path (4xx), which collapses to a single `<unmatched>`
    // series so a 404 scanner cannot explode the cardinality.
    assert_eq!(get(&app, "/no-such-route").await, StatusCode::NOT_FOUND);

    let rendered = handle.render();

    // The counter exists, keyed by the route TEMPLATE and the status CLASS.
    assert!(
        rendered.contains("seiva_http_requests_total"),
        "the request counter must be exposed:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"route="/openapi.json""#) && rendered.contains(r#"status="2xx""#),
        "a matched route is labelled by its template and status class:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"route="<unmatched>""#) && rendered.contains(r#"status="4xx""#),
        "an unmatched path collapses to one bounded series:\n{rendered}"
    );

    // Cardinality guarantees: health probes are exempt and the tenant is never a
    // metric label (it belongs on a span, not in the Prometheus series space).
    assert!(
        !rendered.contains("/health"),
        "health probes must be exempt from metrics:\n{rendered}"
    );
    assert!(
        !rendered.contains("tenant"),
        "tenant must never be a metric label:\n{rendered}"
    );

    // The in-flight gauge and the latency histogram are published too.
    assert!(
        rendered.contains("seiva_http_requests_in_flight"),
        "the in-flight gauge must be exposed:\n{rendered}"
    );
    assert!(
        rendered.contains("seiva_http_request_duration_seconds"),
        "the latency histogram must be exposed:\n{rendered}"
    );
}
