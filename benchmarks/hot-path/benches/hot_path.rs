//! Per-request overhead of the Seiva framework vs plain axum, measured in-process
//! with `tower::ServiceExt::oneshot` — no socket, no database. Two groups:
//!
//! - `light`: a trivial `GET /ping` returning a tiny JSON body. Seiva carries the
//!   always-on base layers (`request_id`, `track_metrics`); axum is bare. Isolates
//!   the fixed per-request cost of the middleware stack (panel finding E4).
//! - `guarded`: a `POST /demo` through `request_id -> track_metrics -> authn ->
//!   permission_guard`, with a handler that extracts `Ctx` (deep-cloning the
//!   `Principal` twice — guard + extractor, panel finding E2) and `Valid<T>`.
//!   axum is a bare route that parses the body and returns 201. The delta is the
//!   framework's zero-trust tax per request.
//!
//! The absolute numbers include a per-iteration `Request` build (same on both
//! sides), so read the Seiva−axum DELTA, not the absolute. Re-run after each
//! optimization to see the gap close.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use criterion::{criterion_group, criterion_main, Criterion};
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt;

use seiva_http::{
    authn, permission_guard, rate_limit, request_id, track_metrics, Authenticator, Authn, Ctx,
    Principal, RateLimit, RateLimiter, RequiredPermission, TokenAuthenticator, Valid, Validate,
    ValidationError,
};

#[derive(Debug, Deserialize)]
struct DemoBody {
    title: String,
}

impl Validate for DemoBody {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}

const TOKEN: &str = "good-token";

fn authenticator() -> TokenAuthenticator {
    TokenAuthenticator::new().with_token(
        TOKEN,
        Principal::in_tenant("user-1", "acme", ["demo.create"]),
    )
}

// --- Seiva apps (the measured framework) -------------------------------------

async fn seiva_ping() -> Response {
    Json(json!({ "ok": true })).into_response()
}

/// Handler on the guarded path: extracting `Ctx` is what deep-clones the
/// `Principal` a second time (the guard already cloned it once) — exactly the
/// cost E2 removes. `Valid<T>` runs deserialization + validation.
async fn seiva_demo(ctx: Ctx, Valid(body): Valid<DemoBody>) -> Response {
    // Touch the principal so the extraction is never optimized away.
    let subject = ctx.principal().subject.clone();
    (
        StatusCode::CREATED,
        Json(json!({ "id": 1, "title": body.title, "by": subject })),
    )
        .into_response()
}

fn seiva_light_app() -> Router {
    Router::new()
        .route("/ping", get(seiva_ping))
        .layer(from_fn(track_metrics))
        .layer(from_fn(request_id))
}

fn seiva_guarded_app() -> Router {
    Router::new()
        .route("/demo", post(seiva_demo))
        .layer(from_fn_with_state(
            RequiredPermission("demo.create"),
            permission_guard,
        ))
        .layer(from_fn_with_state(
            Authn::bearer_only(Arc::new(authenticator()) as Arc<dyn Authenticator>),
            authn,
        ))
        .layer(from_fn(track_metrics))
        .layer(from_fn(request_id))
}

/// The guarded app with the in-process rate limiter mounted — the layer E5
/// optimizes. The budget is effectively unlimited (`u32::MAX`) so the benchmark
/// measures the limiter's **lock/lookup cost under concurrency**, never a 429.
fn seiva_rate_limited_app() -> Router {
    let limiter = RateLimiter::new(RateLimit {
        max_requests: u32::MAX,
        window: Duration::from_secs(3600),
    });
    Router::new()
        .route("/demo", post(seiva_demo))
        .layer(from_fn_with_state(
            RequiredPermission("demo.create"),
            permission_guard,
        ))
        .layer(from_fn_with_state(
            Authn::bearer_only(Arc::new(authenticator()) as Arc<dyn Authenticator>),
            authn,
        ))
        .layer(from_fn_with_state(limiter, rate_limit))
        .layer(from_fn(track_metrics))
        .layer(from_fn(request_id))
}

// --- Plain axum apps (the baseline) ------------------------------------------

async fn axum_ping() -> Response {
    Json(json!({ "ok": true })).into_response()
}

async fn axum_demo(Json(body): Json<DemoBody>) -> Response {
    (
        StatusCode::CREATED,
        Json(json!({ "id": 1, "title": body.title })),
    )
        .into_response()
}

fn axum_light_app() -> Router {
    Router::new().route("/ping", get(axum_ping))
}

fn axum_guarded_app() -> Router {
    Router::new().route("/demo", post(axum_demo))
}

// --- Requests ----------------------------------------------------------------

fn ping_request() -> Request<Body> {
    Request::builder()
        .uri("/ping")
        .body(Body::empty())
        .unwrap()
}

fn demo_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/demo")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"authed"}"#))
        .unwrap()
}

/// A guarded request from a distinct peer IP, so the rate limiter keys each one
/// to a different client (a different shard once E5 lands) — this is what turns
/// the single global `Mutex` into a contention point under concurrency.
fn demo_request_from_ip(i: u32) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/demo")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"authed"}"#))
        .unwrap();
    let ip = IpAddr::V4(Ipv4Addr::from(i.to_be_bytes()));
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(ip, 443)));
    request
}

// --- Harness -----------------------------------------------------------------

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime builds")
}

fn bench_light(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("light");

    let seiva = seiva_light_app();
    group.bench_function("seiva", |b| {
        b.to_async(&rt).iter(|| {
            let app = seiva.clone();
            async move {
                let resp = app.oneshot(ping_request()).await.unwrap();
                assert_eq!(resp.status(), StatusCode::OK);
            }
        });
    });

    let axum = axum_light_app();
    group.bench_function("axum", |b| {
        b.to_async(&rt).iter(|| {
            let app = axum.clone();
            async move {
                let resp = app.oneshot(ping_request()).await.unwrap();
                assert_eq!(resp.status(), StatusCode::OK);
            }
        });
    });

    group.finish();
}

fn bench_guarded(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("guarded");

    let seiva = seiva_guarded_app();
    group.bench_function("seiva", |b| {
        b.to_async(&rt).iter(|| {
            let app = seiva.clone();
            async move {
                let resp = app.oneshot(demo_request()).await.unwrap();
                assert_eq!(resp.status(), StatusCode::CREATED);
            }
        });
    });

    let axum = axum_guarded_app();
    group.bench_function("axum", |b| {
        b.to_async(&rt).iter(|| {
            let app = axum.clone();
            async move {
                let resp = app.oneshot(demo_request()).await.unwrap();
                assert_eq!(resp.status(), StatusCode::CREATED);
            }
        });
    });

    group.finish();
}

/// How many distinct-client requests are fired concurrently per iteration. Large
/// enough that, with one global lock, worker threads serialize on it.
const CONCURRENCY: usize = 64;

fn multi_thread_runtime() -> tokio::runtime::Runtime {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .build()
        .expect("multi-thread runtime builds")
}

/// Fires `CONCURRENCY` guarded requests from distinct IPs at once through the
/// rate-limited app, across worker threads. With one global `Mutex` they contend
/// on it; after E5's sharding, distinct clients hit distinct shards. Read the
/// before/after of THIS group — the other groups don't exercise the limiter.
fn bench_concurrent(c: &mut Criterion) {
    let rt = multi_thread_runtime();
    let app = seiva_rate_limited_app();
    let mut group = c.benchmark_group("concurrent_ratelimit");
    group.sample_size(20);
    group.bench_function("seiva", |b| {
        b.to_async(&rt).iter(|| {
            let app = app.clone();
            async move {
                let mut handles = Vec::with_capacity(CONCURRENCY);
                for i in 0..CONCURRENCY {
                    let app = app.clone();
                    handles.push(tokio::spawn(async move {
                        let resp = app.oneshot(demo_request_from_ip(i as u32)).await.unwrap();
                        debug_assert_eq!(resp.status(), StatusCode::CREATED);
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_light, bench_guarded, bench_concurrent);
criterion_main!(benches);
