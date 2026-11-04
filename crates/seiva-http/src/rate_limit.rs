//! A fixed-window, per-client request rate limiter (OWASP API4).
//!
//! OWASP API Security **API4 (Unrestricted Resource Consumption)** asks that a
//! service bound how much work a single caller can demand. The body-size limit
//! in [`router`](crate::router) bounds one request; this bounds the *rate* of
//! requests from one client.
//!
//! The design follows the framework's "inject the clock" rule (as
//! [`seiva_webhooks`](https://docs.rs) does for its replay window): a
//! [`RateLimiter`] holds a [`Clock`] port, so a test advances time deterministically
//! instead of sleeping. The algorithm is a fixed window — simple, allocation-light,
//! and good enough as a first line of defence; a deployment that needs a smoother
//! token bucket puts a dedicated edge in front.
//!
//! The client key is the peer IP from
//! [`ConnectInfo<SocketAddr>`](axum::extract::ConnectInfo), which the server
//! runtime installs (see `seiva-server`). A request without that extension (a
//! non-networked test harness driving the router directly) shares a single
//! fallback bucket, so the limiter is still exercised but never panics.
//!
//! **Behind a reverse proxy**, the peer IP is the proxy's, so every client would
//! collapse into one bucket. A deployment that terminates TLS at a *single*
//! trusted edge sets [`RateLimiter::trusting_forwarded_for`] (wired from
//! [`HttpSecurity`](crate::HttpSecurity)); the limiter then keys on the rightmost
//! `X-Forwarded-For` entry — the address that trusted proxy observed, which a
//! client cannot spoof (it can only prepend entries to its left). Deeper proxy
//! chains need a dedicated edge, per the token-bucket note above.
//!
//! The per-client table is **bounded**: a flood of distinct source IPs (the very
//! thing API4 guards against) must not let the limiter itself exhaust memory. At
//! [`MAX_TRACKED_CLIENTS`] entries the table first drops every window that has
//! fully elapsed; if it is still full of live counters it fails *open* for the
//! new key rather than evict a live budget or reject a fresh client.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use seiva_ports::Clock;

use crate::problem::Problem;
use crate::request_id::RequestIdExt;

/// How many requests a single client may make per window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    /// The maximum number of requests permitted within one [`window`](Self::window).
    pub max_requests: u32,
    /// The length of the fixed window.
    pub window: Duration,
}

impl RateLimit {
    /// A generous default for a general-purpose JSON API: 300 requests per
    /// minute per client. High enough not to trip a normal UI, low enough to
    /// blunt a single abusive caller; a deployment tunes it from config.
    pub const DEFAULT_MAX_REQUESTS: u32 = 300;
    /// The default window length (60 seconds).
    pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            max_requests: Self::DEFAULT_MAX_REQUESTS,
            window: Self::DEFAULT_WINDOW,
        }
    }
}

/// A [`Clock`] reading the real wall clock. The production default; tests inject
/// a controllable clock instead.
#[derive(Debug, Default, Clone, Copy)]
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// One client's fixed-window counter.
#[derive(Debug, Clone, Copy)]
struct Window {
    start: SystemTime,
    count: u32,
}

/// The ceiling on distinct client keys the limiter tracks at once. Reaching it
/// triggers a sweep of elapsed windows; it bounds the table so the limiter cannot
/// be turned into a memory-exhaustion vector by a flood of distinct source IPs.
/// ~100k small entries is a few MiB.
pub const MAX_TRACKED_CLIENTS: usize = 100_000;

/// A shareable fixed-window rate limiter. Cloning shares the same underlying
/// counters and clock, so the middleware layer and its state stay one limiter.
#[derive(Clone)]
pub struct RateLimiter {
    limit: RateLimit,
    clock: Arc<dyn Clock>,
    windows: Arc<Mutex<HashMap<IpAddr, Window>>>,
    trust_forwarded_for: bool,
}

impl RateLimiter {
    /// A limiter over the real wall clock.
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self::with_clock(limit, Arc::new(SystemClock))
    }

    /// A limiter over an injected [`Clock`], for deterministic tests.
    #[must_use]
    pub fn with_clock(limit: RateLimit, clock: Arc<dyn Clock>) -> Self {
        Self {
            limit,
            clock,
            windows: Arc::new(Mutex::new(HashMap::new())),
            trust_forwarded_for: false,
        }
    }

    /// Keys on the rightmost `X-Forwarded-For` entry instead of the raw peer IP.
    ///
    /// Enable this **only** when the service sits behind a single trusted reverse
    /// proxy that sets `X-Forwarded-For`: the rightmost entry is the address that
    /// proxy observed and a client cannot forge it. Left off (the default), the
    /// limiter keys on the TCP peer — correct for a directly exposed service, but
    /// collapsing every client into one bucket behind a proxy.
    #[must_use]
    pub fn trusting_forwarded_for(mut self, trust: bool) -> Self {
        self.trust_forwarded_for = trust;
        self
    }

    /// Records a request from `key` and reports whether it is permitted.
    ///
    /// Returns `Ok(())` when within the window's budget. When the budget is
    /// exhausted it returns `Err(retry_after)`, the time until the current
    /// window rolls over, without incrementing (so a flood does not keep
    /// pushing the reset further out).
    fn check(&self, key: IpAddr) -> Result<(), Duration> {
        let now = self.clock.now();
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());

        // Bound the table: a flood of distinct keys must not grow it without end.
        if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&key) {
            // Drop every window that has fully elapsed — it carries no live
            // budget, so forgetting it only resets an otherwise-idle key.
            windows.retain(|_, w| {
                now.duration_since(w.start).unwrap_or(Duration::ZERO) < self.limit.window
            });
            // Still full of *live* counters: fail OPEN for this new key rather
            // than evict a live budget (which would hand back free quota) or
            // reject a fresh client (a flood of spoofed IPs would 429 everyone).
            // Memory stays capped either way.
            if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&key) {
                return Ok(());
            }
        }

        let window = windows.entry(key).or_insert(Window {
            start: now,
            count: 0,
        });

        // A clock moving backwards (duration_since errors) is treated as "still
        // in the window": never grants extra budget from a time glitch.
        let elapsed = now.duration_since(window.start).unwrap_or(Duration::ZERO);
        if elapsed >= self.limit.window {
            // The window rolled over: start a fresh one counting this request.
            window.start = now;
            window.count = 1;
            return Ok(());
        }

        if window.count < self.limit.max_requests {
            window.count += 1;
            Ok(())
        } else {
            Err(self.limit.window - elapsed)
        }
    }
}

/// The fallback key for a request with no [`ConnectInfo`] (a direct, non-networked
/// test harness). All such requests share this bucket.
const UNKNOWN_PEER: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// The rightmost parseable `X-Forwarded-For` entry — the address the single
/// trusted reverse proxy in front observed. A client can prepend forged entries
/// to the left of the header, but not past the value the trusted proxy appends,
/// so taking the rightmost resists spoofing. `None` if the header is absent,
/// unreadable, or holds no parseable address.
fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")?
        .to_str()
        .ok()?
        .rsplit(',')
        .find_map(|entry| entry.trim().parse::<IpAddr>().ok())
}

/// The key a request is rate-limited under: the trusted forwarded client IP when
/// `trust_forwarded_for` is set and present, else the TCP peer IP, else the
/// shared [`UNKNOWN_PEER`] fallback.
fn client_key(request: &Request, trust_forwarded_for: bool) -> IpAddr {
    if trust_forwarded_for {
        if let Some(ip) = forwarded_client_ip(request.headers()) {
            return ip;
        }
    }
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(UNKNOWN_PEER)
}

/// Middleware enforcing a [`RateLimiter`] keyed by client IP.
///
/// On an exhausted budget it returns a `429` [`Problem`] carrying a `Retry-After`
/// header (whole seconds, rounded up). Mount it with
/// [`axum::middleware::from_fn_with_state`]`(limiter, rate_limit)`.
pub async fn rate_limit(
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let key = client_key(&request, limiter.trust_forwarded_for);

    match limiter.check(key) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            let request_id = request.extensions().request_id();
            let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
            let mut response = Problem::too_many_requests("request rate limit exceeded")
                .with_instance(request.uri().path().to_owned())
                .with_request_id_opt(request_id)
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert("retry-after", value);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
    use seiva_testing::FixedClock;
    use tower::ServiceExt; // for `oneshot`

    fn app(limiter: RateLimiter) -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(from_fn_with_state(limiter, rate_limit))
    }

    async fn hit(app: &Router) -> StatusCode {
        app.clone()
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn allows_up_to_the_limit_then_429s() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 3,
                window: Duration::from_secs(60),
            },
            clock.clone(),
        );
        let app = app(limiter);

        // Three requests inside the window are permitted...
        assert_eq!(hit(&app).await, StatusCode::OK);
        assert_eq!(hit(&app).await, StatusCode::OK);
        assert_eq!(hit(&app).await, StatusCode::OK);
        // ...the fourth is refused.
        assert_eq!(hit(&app).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn window_rollover_restores_budget() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(60),
            },
            clock.clone(),
        );
        let app = app(limiter);

        assert_eq!(hit(&app).await, StatusCode::OK);
        assert_eq!(hit(&app).await, StatusCode::TOO_MANY_REQUESTS);

        // Advancing past the window resets the budget.
        clock.advance(Duration::from_secs(61));
        assert_eq!(hit(&app).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn too_many_requests_carries_retry_after() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(30),
            },
            clock.clone(),
        );
        let app = app(limiter);

        assert_eq!(hit(&app).await, StatusCode::OK);
        let res = app
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry_after: u64 = res.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(retry_after > 0 && retry_after <= 30);
    }

    #[test]
    fn forwarded_client_ip_takes_the_rightmost_parseable_entry() {
        let mut headers = HeaderMap::new();
        // A client prepends a forged entry; the trusted proxy appends the real
        // one on the right — that is the one we must key on.
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("9.9.9.9, 203.0.113.7"),
        );
        assert_eq!(
            forwarded_client_ip(&headers),
            Some("203.0.113.7".parse().unwrap())
        );

        assert_eq!(forwarded_client_ip(&HeaderMap::new()), None);
    }

    #[test]
    fn trusted_forwarded_for_keys_each_client_independently() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(60),
            },
            clock.clone(),
        )
        .trusting_forwarded_for(true);

        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let b: IpAddr = "203.0.113.2".parse().unwrap();

        // One client exhausting its budget does not touch another's — the bug a
        // single shared (proxy-IP) bucket would cause.
        assert!(limiter.check(a).is_ok());
        assert!(limiter.check(a).is_err());
        assert!(limiter.check(b).is_ok());
    }

    #[test]
    fn the_client_table_is_bounded_under_a_flood_of_distinct_keys() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(RateLimit::default(), clock.clone());

        // Far more distinct keys than the cap, all within one window, must not
        // grow the table past the ceiling: the limiter cannot be the API4 vector
        // it defends against.
        for i in 0..(MAX_TRACKED_CLIENTS as u64 + 5_000) {
            let octets = (i >> 24) as u8;
            let ip = IpAddr::from([octets, (i >> 16) as u8, (i >> 8) as u8, i as u8]);
            let _ = limiter.check(ip);
        }
        let tracked = limiter.windows.lock().unwrap().len();
        assert!(
            tracked <= MAX_TRACKED_CLIENTS,
            "table must stay bounded, got {tracked}"
        );
    }
}
