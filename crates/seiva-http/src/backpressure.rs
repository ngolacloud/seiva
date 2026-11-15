//! Per-route load control: a request **timeout** (→ `504 Gateway Timeout`) and a
//! **concurrency limit with immediate shedding** (→ `503 Service Unavailable`),
//! applied per route so one slow or heavy endpoint cannot pin every worker while
//! the rest of the service stays responsive.
//!
//! # Why this exists (OWASP API4)
//!
//! The framework already bounds *one* request's body size ([`router`](crate::router))
//! and the *rate* of requests from one client ([`rate_limit`](mod@crate::rate_limit),
//! `429`). Neither bounds the **work in flight** for a single route: a reporting
//! endpoint that takes seconds, or a write that stalls on a slow dependency, can
//! accumulate requests until the whole pool is blocked and healthy routes start
//! failing too. OWASP API Security **API4 (Unrestricted Resource Consumption)**
//! asks the service to bound that. This primitive does, per route:
//!
//! - **Timeout.** A request to a controlled route that outlives its configured
//!   budget is cut off with `504` — distinct from the framework's global
//!   `408 Request Timeout` (a slowloris guard on the whole service). The `504`
//!   says "this route's own deadline was exceeded", the operator-facing meaning
//!   of a gateway deadline.
//! - **Backpressure.** Each in-flight request to a controlled route holds one
//!   permit of a fixed-size [`Semaphore`]. At capacity the next request is shed
//!   **immediately** (`try_acquire`, never queued) with `503` — fail fast rather
//!   than let work pile up unbounded. A `503` is the load balancer's cue to
//!   retry elsewhere or back off, and it keeps the route's latency bounded for
//!   the requests already admitted.
//!
//! # Per route, composed at the root
//!
//! A [`RouteControls`] maps path prefixes to [`RouteLimit`]s (longest prefix
//! wins), and [`with_route_controls`] wraps a router in the middleware that
//! enforces them. The composition root applies it to the **module** sub-router
//! before the framework layers wrap it, so control runs *after* the caller is
//! authenticated (an admitted caller) but *around* each route's own work. A route
//! with no matching rule is untouched — the control is opt-in per endpoint, the
//! tower-level counterpart to the opt-in [`DistributedRateLimit`](crate::DistributedRateLimit).
//!
//! ```no_run
//! use std::time::Duration;
//! use seiva_http::{with_route_controls, RouteControls, RouteLimit};
//!
//! # let modules = axum::Router::new();
//! // Writes under `/v1/projects` must answer within 2 s and run at most 64 at a
//! // time; past either bound the caller gets a `504`/`503` instead of a stuck worker.
//! let controls = RouteControls::new().route(
//!     "/v1/projects",
//!     RouteLimit::new().timeout(Duration::from_secs(2)).max_in_flight(64),
//! );
//! let modules = with_route_controls(modules, controls);
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::sync::Semaphore;

use crate::problem::{Problem, ProblemResponse};
use crate::request_id::RequestIdExt;

/// The `Retry-After` (seconds) a shed `503` suggests before retrying. Small on
/// purpose: a concurrency breach is transient — the permit frees the moment an
/// in-flight request finishes.
const BACKPRESSURE_RETRY_AFTER: HeaderValue = HeaderValue::from_static("1");

/// The load-control limits for one route: an optional per-request `timeout`
/// (breach → `504`) and an optional `max_in_flight` concurrency cap (breach →
/// `503`). Either, both, or neither; an empty `RouteLimit` is a no-op.
#[derive(Debug, Clone, Default)]
pub struct RouteLimit {
    timeout: Option<Duration>,
    max_in_flight: Option<usize>,
}

impl RouteLimit {
    /// An empty limit (no timeout, no concurrency cap). Build it up with
    /// [`RouteLimit::timeout`] / [`RouteLimit::max_in_flight`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cut a request off with `504` if it outlives `budget`.
    #[must_use]
    pub fn timeout(mut self, budget: Duration) -> Self {
        self.timeout = Some(budget);
        self
    }

    /// Admit at most `max` concurrent requests to this route; shed the rest with
    /// `503` (fail fast, never queue). `0` is clamped to `1` so a route is never
    /// wedged shut by a typo.
    #[must_use]
    pub fn max_in_flight(mut self, max: usize) -> Self {
        self.max_in_flight = Some(max.max(1));
        self
    }
}

/// One compiled rule: a path prefix, its timeout, and — if a concurrency cap was
/// set — the shared semaphore that enforces it across every request to the route.
#[derive(Clone)]
struct Rule {
    prefix: String,
    timeout: Option<Duration>,
    semaphore: Option<Arc<Semaphore>>,
}

/// A set of per-route load-control rules. Each [`route`](RouteControls::route)
/// adds a prefix → [`RouteLimit`] mapping; at request time the **longest**
/// matching prefix wins, so a specific `/v1/projects/export` rule overrides a
/// broad `/v1` one. Cheap to clone (the semaphores are shared behind `Arc`).
#[derive(Clone, Default)]
pub struct RouteControls {
    rules: Vec<Rule>,
}

impl RouteControls {
    /// An empty rule set — [`with_route_controls`] with this is a no-op.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply `limit` to every request whose path starts with `prefix`. The
    /// concurrency cap (if any) is materialised into one shared semaphore here,
    /// so all requests to the route draw from the SAME budget.
    #[must_use]
    pub fn route(mut self, prefix: impl Into<String>, limit: RouteLimit) -> Self {
        self.rules.push(Rule {
            prefix: prefix.into(),
            timeout: limit.timeout,
            semaphore: limit.max_in_flight.map(|n| Arc::new(Semaphore::new(n))),
        });
        // Longest prefix first, so `match_rule` returns the most specific rule.
        self.rules
            .sort_by_key(|r| std::cmp::Reverse(r.prefix.len()));
        self
    }

    /// Whether any rule is configured (an empty set means no control at all).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The most specific rule whose prefix the `path` starts with, if any.
    fn match_rule(&self, path: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| path.starts_with(&r.prefix))
    }
}

/// Wrap `router` in the per-route load control described by `controls`. An empty
/// `controls` returns `router` untouched (no layer, no cost). Apply it to the
/// module sub-router in the composition root, before [`router_with_modules`](crate::router_with_modules)
/// wraps the framework layers around it.
pub fn with_route_controls(router: Router, controls: RouteControls) -> Router {
    if controls.is_empty() {
        return router;
    }
    router.layer(from_fn_with_state(Arc::new(controls), route_controls))
}

/// The middleware: match the request path to a rule, hold a concurrency permit
/// (shedding with `503` when the route is saturated), and bound the handler with
/// the route's timeout (`504` on breach). A path with no rule passes straight
/// through.
async fn route_controls(
    State(controls): State<Arc<RouteControls>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let Some(rule) = controls.match_rule(&path) else {
        return next.run(request).await;
    };
    let request_id = request.extensions().request_id();

    // Backpressure: one permit per in-flight request. `try_acquire` never
    // queues, so a saturated route sheds the overflow immediately with `503`
    // rather than letting latency grow unbounded. The permit is held for the
    // whole handling (including the timeout wait) and released on drop.
    let _permit = match &rule.semaphore {
        Some(semaphore) => match Arc::clone(semaphore).try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => return overloaded_response(&path, request_id),
        },
        None => None,
    };

    match rule.timeout {
        Some(budget) => match tokio::time::timeout(budget, next.run(request)).await {
            Ok(response) => response,
            Err(_) => gateway_timeout_response(&path, request_id),
        },
        None => next.run(request).await,
    }
}

/// The `503` a shed request gets when its route is at capacity, carrying a short
/// `Retry-After` (the breach is transient).
fn overloaded_response(path: &str, request_id: Option<String>) -> Response {
    let mut response = ProblemResponse(
        Problem::new(StatusCode::SERVICE_UNAVAILABLE, "service overloaded")
            .with_detail("the route is at its concurrency limit; retry shortly")
            .with_instance(path.to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response();
    response
        .headers_mut()
        .insert("retry-after", BACKPRESSURE_RETRY_AFTER);
    response
}

/// The `504` a request gets when it outlives its route's timeout budget.
fn gateway_timeout_response(path: &str, request_id: Option<String>) -> Response {
    ProblemResponse(
        Problem::new(StatusCode::GATEWAY_TIMEOUT, "route timed out")
            .with_detail("the route did not complete within its configured timeout")
            .with_instance(path.to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tokio::sync::Notify;
    use tower::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn request_over_the_budget_returns_504() {
        let app = with_route_controls(
            Router::new().route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    "done"
                }),
            ),
            RouteControls::new()
                .route("/slow", RouteLimit::new().timeout(Duration::from_millis(5))),
        );
        let res = app
            .oneshot(HttpRequest::get("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn request_within_the_budget_passes() {
        let app = with_route_controls(
            Router::new().route("/fast", get(|| async { "done" })),
            RouteControls::new().route("/fast", RouteLimit::new().timeout(Duration::from_secs(5))),
        );
        let res = app
            .oneshot(HttpRequest::get("/fast").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unmatched_path_is_not_controlled() {
        let app = with_route_controls(
            Router::new().route(
                "/other",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    "done"
                }),
            ),
            // A rule only for `/slow`; `/other` must be untouched despite being slow.
            RouteControls::new()
                .route("/slow", RouteLimit::new().timeout(Duration::from_millis(1))),
        );
        let res = app
            .oneshot(HttpRequest::get("/other").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn concurrency_limit_sheds_overflow_with_503() {
        // `/held` parks on a Notify until the test releases it, so the first
        // request deterministically holds the only permit while the second tries.
        let gate = Arc::new(Notify::new());
        let entered = Arc::new(Notify::new());
        let app = {
            let gate = Arc::clone(&gate);
            let entered = Arc::clone(&entered);
            with_route_controls(
                Router::new().route(
                    "/held",
                    get(move || {
                        let gate = Arc::clone(&gate);
                        let entered = Arc::clone(&entered);
                        async move {
                            entered.notify_one();
                            gate.notified().await;
                            "done"
                        }
                    }),
                ),
                RouteControls::new().route("/held", RouteLimit::new().max_in_flight(1)),
            )
        };

        // Request 1 enters and holds the permit, parked on the gate.
        let first = {
            let app = app.clone();
            tokio::spawn(async move {
                app.oneshot(HttpRequest::get("/held").body(Body::empty()).unwrap())
                    .await
                    .unwrap()
            })
        };
        entered.notified().await; // the handler is in → permit is taken

        // Request 2 finds no permit and is shed with 503.
        let second = app
            .clone()
            .oneshot(HttpRequest::get("/held").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            second
                .headers()
                .get("retry-after")
                .map(|v| v.to_str().unwrap()),
            Some("1")
        );

        // Release request 1; it completes 200 and frees the permit.
        gate.notify_one();
        let first = first.await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
    }
}
