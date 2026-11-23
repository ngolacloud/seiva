//! A fixed-window, per-client request rate limiter for the MCP endpoint (OWASP
//! API4 — Unrestricted Resource Consumption).
//!
//! The body-size limit on [`router`](crate::router) bounds *one* request; this
//! bounds the *rate* of requests from one client, so a single caller cannot pin
//! the executor (and DORM behind it) with a burst. It is deliberately small and
//! self-contained: `seiva-http`'s richer limiter (tenant dimension, injected
//! clock, distributed store) lives behind its whole framework graph, which this
//! DORM-coupled transport does not pull in — a smaller in-process window is the
//! right first line of defence here, and a deployment that needs more puts a
//! dedicated edge in front.
//!
//! The client key is the peer IP from
//! [`ConnectInfo<SocketAddr>`](axum::extract::ConnectInfo), which
//! [`serve`](crate::serve) installs. A request without it (a test harness driving
//! the router directly) shares one fallback bucket, so the limiter is still
//! exercised but never panics. The per-client table is **bounded**
//! ([`MAX_TRACKED_CLIENTS`]): a flood of distinct source IPs — the very thing
//! API4 guards against — must not turn the limiter into its own memory-exhaustion
//! vector, so at the cap it first drops elapsed windows and then evicts the
//! oldest live one rather than failing open.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The fallback key for a request with no [`ConnectInfo`] (a direct, non-networked
/// test harness). All such requests share this bucket.
const UNKNOWN_PEER: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// The ceiling on distinct client keys tracked at once, so a flood of distinct
/// source IPs cannot grow the table without end. ~100k small entries is a few MiB.
pub const MAX_TRACKED_CLIENTS: usize = 100_000;

/// One client's fixed-window counter.
#[derive(Clone, Copy)]
struct ClientWindow {
    start: Instant,
    count: u32,
}

/// A shareable fixed-window rate limiter keyed by peer IP. Cloning shares the same
/// underlying counters, so the middleware layer and its state stay one limiter.
#[derive(Clone)]
pub(crate) struct RateLimiter {
    max_requests: u32,
    window: Duration,
    windows: Arc<Mutex<HashMap<IpAddr, ClientWindow>>>,
}

impl RateLimiter {
    /// A limiter permitting `max_requests` per `window` per peer IP.
    pub(crate) fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Records a request from `ip` and reports whether it is permitted. On an
    /// exhausted budget it returns `Err(retry_after)` without incrementing, so a
    /// flood does not keep pushing the window's reset further out.
    fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());

        // Bound the table: a flood of distinct keys must not grow it without end.
        if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&ip) {
            // Drop fully-elapsed windows first — they carry no live budget.
            windows.retain(|_, w| now.duration_since(w.start) < self.window);
            // Still full of live counters: evict the oldest (closest to rolling
            // over anyway) rather than failing open or rejecting the fresh client.
            if windows.len() >= MAX_TRACKED_CLIENTS && !windows.contains_key(&ip) {
                if let Some(oldest) = windows.iter().min_by_key(|(_, w)| w.start).map(|(k, _)| *k) {
                    windows.remove(&oldest);
                }
            }
        }

        let window = windows.entry(ip).or_insert(ClientWindow {
            start: now,
            count: 0,
        });
        let elapsed = now.duration_since(window.start);
        if elapsed >= self.window {
            // The window rolled over: start a fresh one counting this request.
            window.start = now;
            window.count = 1;
            return Ok(());
        }
        if window.count < self.max_requests {
            window.count += 1;
            Ok(())
        } else {
            Err(self.window - elapsed)
        }
    }
}

/// Middleware enforcing a [`RateLimiter`] keyed by peer IP. On an exhausted budget
/// it returns `429 Too Many Requests` with a `Retry-After` header (whole seconds,
/// rounded up). Mount with [`axum::middleware::from_fn_with_state`]`(limiter, rate_limit)`.
pub(crate) async fn rate_limit(
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(UNKNOWN_PEER, |ci| ci.0.ip());

    match limiter.check(ip) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, seconds.to_string())],
                "request rate limit exceeded",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_up_to_the_limit_then_refuses() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(limiter.check(ip).is_ok());
        assert!(limiter.check(ip).is_ok());
        // The third within the window is over budget.
        assert!(limiter.check(ip).is_err());
    }

    #[test]
    fn distinct_clients_keep_independent_budgets() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let b: IpAddr = "203.0.113.2".parse().unwrap();
        assert!(limiter.check(a).is_ok());
        assert!(limiter.check(a).is_err());
        // One client exhausting its budget does not touch another's.
        assert!(limiter.check(b).is_ok());
    }
}
