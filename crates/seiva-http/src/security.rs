//! Secure-by-default response headers and configurable CORS.
//!
//! The framework's threat model treats a Seiva service as a JSON API sitting
//! behind a TLS-terminating edge. Two transport-level controls address OWASP API
//! Security **API8 (Security Misconfiguration)**:
//!
//! - [`security_headers`] — a tower middleware that adds a conservative set of
//!   response headers (content-type sniffing off, framing denied, a locked-down
//!   Content-Security-Policy, HSTS) to **every** response, problems included. It
//!   only ever adds a header that is *absent*, so a handler that deliberately set
//!   its own value (a UI route relaxing the CSP, say) keeps it — no hidden
//!   override.
//! - [`cors_layer`] — builds a [`CorsLayer`] from an explicit allowlist of
//!   origins ([`HttpSecurity::cors_allowed_origins`]). An **empty** allowlist
//!   means no CORS layer at all, i.e. browsers enforce same-origin — the secure
//!   default. A cross-origin caller is admitted only when its `Origin` is on the
//!   list.
//!
//! Neither control changes a same-origin, non-browser caller's behaviour beyond
//! the added response headers, so wiring them into [`router`](crate::router) does
//! not break an existing request path.

use axum::http::header::{
    HeaderName, HeaderValue, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::Method;
use axum::{extract::Request, middleware::Next, response::Response};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::rate_limit::RateLimit;

/// `X-XSS-Protection`, set to `0` to disable the legacy, deprecated and
/// footgun-prone browser XSS auditor (current guidance; the CSP does the real
/// work). Not a typed constant in `http`, so named here.
static X_XSS_PROTECTION: HeaderName = HeaderName::from_static("x-xss-protection");

/// The transport's security configuration: the knobs a deployment turns that are
/// not safe to hard-code. Read from the environment by the server runtime (see
/// `seiva-server`) and handed to [`router_with_security`](crate::router_with_security).
///
/// [`Default`] is the secure baseline: **no** cross-origin access (same-origin
/// only) and rate limiting **on** at [`RateLimit::default`]. A deployment opts
/// origins in and tunes (or disables) the limit explicitly.
#[derive(Debug, Clone)]
pub struct HttpSecurity {
    /// The exact origins allowed to make cross-origin (CORS) requests, e.g.
    /// `https://app.example.com`. Empty means same-origin only: [`cors_layer`]
    /// returns `None` and no `Access-Control-Allow-Origin` is ever emitted.
    pub cors_allowed_origins: Vec<String>,
    /// The per-client request rate limit, or `None` to disable rate limiting.
    /// `Some(RateLimit::default())` by default — disabling it is an explicit
    /// choice, never the default.
    pub rate_limit: Option<RateLimit>,
    /// Whether the rate limiter keys on the trusted `X-Forwarded-For` client IP
    /// instead of the raw TCP peer. Leave `false` (the default) for a directly
    /// exposed service; set it **only** behind a single trusted reverse proxy, or
    /// every client collapses into the proxy's single bucket. See
    /// [`RateLimiter::trusting_forwarded_for`](crate::RateLimiter::trusting_forwarded_for).
    pub trust_forwarded_for: bool,
}

impl Default for HttpSecurity {
    fn default() -> Self {
        Self {
            cors_allowed_origins: Vec::new(),
            rate_limit: Some(RateLimit::default()),
            trust_forwarded_for: false,
        }
    }
}

impl HttpSecurity {
    /// A configuration allowing the given origins for CORS (rate limiting left at
    /// the default). Each origin must be a full `scheme://host[:port]`, compared
    /// verbatim against the request's `Origin` header.
    #[must_use]
    pub fn with_cors_origins<I, S>(origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            cors_allowed_origins: origins.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }
}

/// Adds a secure-by-default header only when the response does not already carry
/// it, so a handler's deliberate choice is never silently overridden.
fn set_if_absent(response: &mut Response, name: HeaderName, value: &'static str) {
    let headers = response.headers_mut();
    if !headers.contains_key(&name) {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

/// Middleware that adds the framework's default security response headers.
///
/// Applied to every response (success and [`Problem`](crate::Problem) alike):
///
/// - `X-Content-Type-Options: nosniff` — no MIME sniffing.
/// - `X-Frame-Options: DENY` and `Content-Security-Policy: default-src 'none';
///   frame-ancestors 'none'` — the response is not framed and loads nothing; a
///   JSON API needs no richer policy, and a route that does sets its own first.
/// - `Referrer-Policy: no-referrer` — no referrer leakage.
/// - `Strict-Transport-Security: max-age=63072000; includeSubDomains` — HSTS,
///   meaningful once served over TLS (the assumed edge); a browser ignores it
///   over plain HTTP, so it is harmless in local development.
/// - `X-XSS-Protection: 0` — disables the deprecated legacy auditor.
///
/// Mount it with [`axum::middleware::from_fn`].
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    set_if_absent(&mut response, X_CONTENT_TYPE_OPTIONS, "nosniff");
    set_if_absent(&mut response, X_FRAME_OPTIONS, "DENY");
    set_if_absent(
        &mut response,
        CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'",
    );
    set_if_absent(&mut response, REFERRER_POLICY, "no-referrer");
    set_if_absent(
        &mut response,
        STRICT_TRANSPORT_SECURITY,
        "max-age=63072000; includeSubDomains",
    );
    set_if_absent(&mut response, X_XSS_PROTECTION.clone(), "0");
    response
}

/// Builds the CORS layer from the configured allowlist, or `None` when the
/// allowlist is empty (same-origin only — the secure default).
///
/// When non-empty, only an exact `Origin` on the list is reflected back in
/// `Access-Control-Allow-Origin`; every other origin is left without CORS
/// headers, so the browser blocks the cross-origin read. A malformed origin in
/// the list is skipped (it can never match a real `Origin` header anyway).
#[must_use]
pub fn cors_layer(security: &HttpSecurity) -> Option<CorsLayer> {
    if security.cors_allowed_origins.is_empty() {
        return None;
    }
    let origins: Vec<HeaderValue> = security
        .cors_allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    if origins.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{router_with_security, Health};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    fn ready_app(security: &HttpSecurity) -> axum::Router {
        let health = Health::starting();
        health.set_ready();
        router_with_security(health, security)
    }

    #[tokio::test]
    async fn security_headers_are_present_on_every_response() {
        let app = ready_app(&HttpSecurity::default());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let headers = res.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert!(headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'"));
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert!(headers.contains_key("strict-transport-security"));
        assert_eq!(headers["x-xss-protection"], "0");
    }

    #[tokio::test]
    async fn default_posture_emits_no_cors_header() {
        // Same-origin only: an allowlisted origin is required for any
        // Access-Control-Allow-Origin to appear.
        assert!(cors_layer(&HttpSecurity::default()).is_none());
        let app = ready_app(&HttpSecurity::default());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!res.headers().contains_key("access-control-allow-origin"));
    }

    #[tokio::test]
    async fn cors_reflects_an_allowlisted_origin_only() {
        let security = HttpSecurity::with_cors_origins(["https://app.example.com"]);
        let app = ready_app(&security);

        // The allowlisted origin is reflected back.
        let allowed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .header("origin", "https://app.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            allowed.headers()["access-control-allow-origin"],
            "https://app.example.com"
        );

        // An origin NOT on the list gets no CORS header, so the browser blocks it.
        let blocked = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!blocked
            .headers()
            .contains_key("access-control-allow-origin"));
    }
}
