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
//! The client key carries a **tenant dimension** (OWASP API4, multi-tenant
//! isolation): an authenticated caller is keyed by `(tenant, peer IP)`, so two
//! tenants sharing a source IP (a corporate NAT, a mobile carrier, a cloud egress)
//! get **separate budgets** — one tenant exhausting its rate can no longer `429`
//! another from the same address. An anonymous caller is keyed by peer IP alone.
//!
//! The per-client table is **bounded**: a flood of distinct keys (the very thing
//! API4 guards against) must not let the limiter itself exhaust memory. At
//! [`MAX_TRACKED_CLIENTS`] entries the table first drops every window that has
//! fully elapsed; if it is still full of live counters it **evicts the oldest
//! window** to make room for the new key. It does *not* fail open (which would let
//! a flood switch the limiter off globally — a bypass of the very control) nor
//! reject the fresh client (which would turn the flood into a `429` DoS for
//! everyone): memory stays capped while the limit keeps applying.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use seiva_identity::Principal;
use seiva_ports::Clock;

use crate::problem::{Problem, ProblemResponse};
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

/// The identity a request is rate-limited under. An authenticated caller is
/// scoped by its tenant **and** peer IP, so tenants behind one shared address keep
/// independent budgets (multi-tenant API4 isolation); an unauthenticated caller is
/// scoped by peer IP alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ClientKey {
    /// Authenticated caller: `(tenant, peer IP)`.
    Tenant { tenant: String, ip: IpAddr },
    /// Unauthenticated (or untenanted) caller: peer IP only.
    Ip(IpAddr),
}

impl From<IpAddr> for ClientKey {
    fn from(ip: IpAddr) -> Self {
        ClientKey::Ip(ip)
    }
}

impl std::fmt::Display for ClientKey {
    /// The stable string the distributed store keys on. The `t:`/`ip:` prefixes
    /// keep a tenant named like an IP from colliding with an IP bucket.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientKey::Tenant { tenant, ip } => write!(f, "t:{tenant}:{ip}"),
            ClientKey::Ip(ip) => write!(f, "ip:{ip}"),
        }
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
    windows: Arc<Mutex<HashMap<ClientKey, Window>>>,
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
    fn check(&self, key: ClientKey) -> Result<(), Duration> {
        let now = self.clock.now();
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());

        // Bound the table: a flood of distinct keys must not grow it without end.
        if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&key) {
            // Drop every window that has fully elapsed — it carries no live
            // budget, so forgetting it only resets an otherwise-idle key.
            windows.retain(|_, w| {
                now.duration_since(w.start).unwrap_or(Duration::ZERO) < self.limit.window
            });
            // Still full of *live* counters: evict the oldest window (closest to
            // rolling over anyway) to make room for the new key. Not fail-open
            // (which switches the whole limiter off under a flood — a bypass of the
            // control) and not a blanket reject (which turns the flood into a `429`
            // DoS for fresh clients); memory stays capped while the limit keeps
            // applying to every key, new ones included.
            if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&key) {
                if let Some(oldest) = windows
                    .iter()
                    .min_by_key(|(_, w)| w.start)
                    .map(|(k, _)| k.clone())
                {
                    windows.remove(&oldest);
                }
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

/// Whether `path` is exempt from rate limiting — the framework's health probes
/// (`/health/live`, `/health/ready`), which a load balancer polls constantly and
/// must never be throttled out of service.
fn is_rate_limit_exempt(path: &str) -> bool {
    path == "/health/live" || path == "/health/ready"
}

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

/// The key a request is rate-limited under. The peer IP is the trusted forwarded
/// client IP when `trust_forwarded_for` is set and present, else the TCP peer IP,
/// else the shared [`UNKNOWN_PEER`] fallback. When the request carries an
/// authenticated tenant (the `authn` layer runs outside the limiter and installs
/// the [`Principal`]), the key is scoped by `(tenant, IP)` so tenants sharing an
/// address keep separate budgets; otherwise it is the IP alone.
fn client_key(request: &Request, trust_forwarded_for: bool) -> ClientKey {
    let ip = if trust_forwarded_for {
        forwarded_client_ip(request.headers())
    } else {
        None
    }
    .or_else(|| {
        request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
    })
    .unwrap_or(UNKNOWN_PEER);

    match request
        .extensions()
        .get::<Principal>()
        .and_then(|p| p.tenant.clone())
    {
        Some(tenant) => ClientKey::Tenant { tenant, ip },
        None => ClientKey::Ip(ip),
    }
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
    // Operational health probes are exempt (§8): a load balancer or orchestrator
    // polls `/health/*` constantly, and many clients can share one source IP
    // (a NAT, the proxy itself). Counting probes against a client budget would
    // let that traffic trip a `429` and get the instance marked unhealthy — a
    // restart storm. The probes carry no business effect, so they skip the limiter.
    if is_rate_limit_exempt(request.uri().path()) {
        return next.run(request).await;
    }

    let key = client_key(&request, limiter.trust_forwarded_for);

    match limiter.check(key) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            let request_id = request.extensions().request_id();
            rate_limited_response(retry_after, request.uri().path(), request_id)
        }
    }
}

/// The `429` problem response carrying a `Retry-After` header (whole seconds,
/// rounded up) — shared by the in-process and the distributed limiters.
fn rate_limited_response(
    retry_after: Duration,
    path: &str,
    request_id: Option<String>,
) -> Response {
    let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
    let mut response = ProblemResponse(
        Problem::too_many_requests("request rate limit exceeded")
            .with_instance(path.to_owned())
            .with_request_id_opt(request_id),
    )
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert("retry-after", value);
    }
    response
}

/// A **distributed** rate limiter: the same window + proxy-trust policy as the
/// in-process [`RateLimiter`], but the budget lives in a shared
/// [`RateLimitStore`](seiva_ports::RateLimitStore) (e.g. Redis) so it holds
/// across replicas (ADR 0015). Build one and mount [`rate_limit_with_store`].
#[derive(Clone)]
pub struct DistributedRateLimit {
    store: std::sync::Arc<dyn seiva_ports::RateLimitStore>,
    limit: RateLimit,
    trust_forwarded_for: bool,
}

impl DistributedRateLimit {
    /// A distributed limiter over `store`, enforcing `limit` per client.
    #[must_use]
    pub fn new(store: std::sync::Arc<dyn seiva_ports::RateLimitStore>, limit: RateLimit) -> Self {
        Self {
            store,
            limit,
            trust_forwarded_for: false,
        }
    }

    /// Keys on the rightmost `X-Forwarded-For` entry (single trusted edge only),
    /// exactly as [`RateLimiter::trusting_forwarded_for`].
    #[must_use]
    pub fn trusting_forwarded_for(mut self, trust: bool) -> Self {
        self.trust_forwarded_for = trust;
        self
    }
}

/// Middleware enforcing a [`DistributedRateLimit`]. Health probes are exempt (as
/// in [`rate_limit`]). The store check is atomic in the backend; if the store
/// errors (a transient outage) the request is permitted — rate limiting is
/// abuse-dampening, so a limiter outage must not drop legitimate traffic.
pub async fn rate_limit_with_store(
    State(cfg): State<DistributedRateLimit>,
    request: Request,
    next: Next,
) -> Response {
    if is_rate_limit_exempt(request.uri().path()) {
        return next.run(request).await;
    }
    let key = client_key(&request, cfg.trust_forwarded_for).to_string();
    match cfg
        .store
        .check(&key, cfg.limit.max_requests, cfg.limit.window)
        .await
    {
        Ok(seiva_ports::RateLimitDecision::Permitted) => next.run(request).await,
        Ok(seiva_ports::RateLimitDecision::Limited { retry_after }) => {
            let request_id = request.extensions().request_id();
            rate_limited_response(retry_after, request.uri().path(), request_id)
        }
        Err(error) => {
            tracing::warn!(target: "seiva::rate_limit", %error, "rate-limit store failed; failing open");
            next.run(request).await
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
    async fn store_backed_limiter_permits_up_to_the_limit_then_429s_and_exempts_health() {
        use seiva_testing::InMemoryRateLimitStore;
        let store = std::sync::Arc::new(InMemoryRateLimitStore::new());
        let cfg = DistributedRateLimit::new(
            store,
            RateLimit {
                max_requests: 2,
                window: Duration::from_secs(60),
            },
        );
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/health/ready", get(|| async { "ready" }))
            .layer(from_fn_with_state(cfg, rate_limit_with_store));

        let hit = |path: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(
                    HttpRequest::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };

        // Two permitted, the third over budget.
        assert_eq!(hit("/").await, StatusCode::OK);
        assert_eq!(hit("/").await, StatusCode::OK);
        assert_eq!(hit("/").await, StatusCode::TOO_MANY_REQUESTS);
        // Health stays exempt even when the business budget is spent.
        assert_eq!(hit("/health/ready").await, StatusCode::OK);
        assert_eq!(hit("/health/ready").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn health_probes_are_exempt_from_the_limit() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(60),
            },
            clock.clone(),
        );
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/health/live", get(|| async { "live" }))
            .route("/health/ready", get(|| async { "ready" }))
            .layer(from_fn_with_state(limiter, rate_limit));

        let probe = |path: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(
                    HttpRequest::builder()
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };

        // Hammer the probes well past the budget of 1 — never throttled.
        for _ in 0..5 {
            assert_eq!(probe("/health/live").await, StatusCode::OK);
            assert_eq!(probe("/health/ready").await, StatusCode::OK);
        }
        // A business route still consumes and exhausts the same small budget.
        assert_eq!(probe("/").await, StatusCode::OK);
        assert_eq!(probe("/").await, StatusCode::TOO_MANY_REQUESTS);
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
        assert!(limiter.check(a.into()).is_ok());
        assert!(limiter.check(a.into()).is_err());
        assert!(limiter.check(b.into()).is_ok());
    }

    #[test]
    fn tenants_sharing_an_ip_get_independent_budgets() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(60),
            },
            clock.clone(),
        );
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let acme = ClientKey::Tenant {
            tenant: "acme".to_owned(),
            ip,
        };
        let globex = ClientKey::Tenant {
            tenant: "globex".to_owned(),
            ip,
        };

        // Same source IP, different tenants: `acme` spending its budget must not
        // `429` `globex` (the noisy-neighbour bug a per-IP-only key would cause).
        assert!(limiter.check(acme.clone()).is_ok());
        assert!(limiter.check(acme).is_err());
        assert!(limiter.check(globex).is_ok());
    }

    #[test]
    fn a_saturated_table_evicts_the_oldest_rather_than_failing_open() {
        let clock = Arc::new(FixedClock::at_unix_secs(0));
        let limiter = RateLimiter::with_clock(
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(3600),
            },
            clock.clone(),
        );
        // Fill the table with live counters (none elapsed within the long window).
        for i in 0..(MAX_TRACKED_CLIENTS as u64) {
            let ip = IpAddr::from([(i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8, i as u8]);
            assert!(limiter.check(ip.into()).is_ok());
        }
        // A brand-new key under saturation is still tracked (not waved through): it
        // consumes its single-request budget and the next hit is refused, proving
        // the limiter did not fail open.
        let fresh: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(limiter.check(fresh.into()).is_ok());
        assert!(limiter.check(fresh.into()).is_err());
        // And the table stayed bounded.
        assert!(limiter.windows.lock().unwrap().len() <= MAX_TRACKED_CLIENTS);
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
            let _ = limiter.check(ip.into());
        }
        let tracked = limiter.windows.lock().unwrap().len();
        assert!(
            tracked <= MAX_TRACKED_CLIENTS,
            "table must stay bounded, got {tracked}"
        );
    }
}
