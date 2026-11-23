//! The MCP HTTP transport Seiva owns, in front of DORM's protocol core.
//!
//! One endpoint, `POST /mcp`, that in order:
//! 1. **authenticates first** — extracts the `Authorization: Bearer <token>`
//!    header and runs it through the [`Authenticator`]; a missing or invalid
//!    credential is rejected (`401`) and a forbidden one (`403`) before the body
//!    is read or DORM is touched. This is the zero-trust gate — deny by default.
//! 2. builds a **per-request** [`dorm_mcp::ServerConfig`] from the base config,
//!    overriding `tenant` with the principal's tenant and narrowing `writable` to
//!    `base.writable && can_write(principal)` (the write permission on the shared
//!    `Principal`), so the tenant and write scope come from the authenticated
//!    principal, never from a tool argument;
//! 3. hands the raw JSON-RPC request line to [`dorm_mcp::handle_message`] — the
//!    protocol core DORM owns — and returns its response as `application/json`
//!    (`200`), or `204 No Content` when it is a notification (no response).
//!
//! DORM owns the MCP protocol (`handle_message`) and the `stdio` transport; this
//! module is purely the HTTP skin plus Seiva's auth. It does not reimplement any
//! method dispatch, and it does not call DORM's own `serve_http` / `serve_on`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use dorm_mcp::ServerConfig;

use crate::auth::{can_write, AuthError, Authenticator};
use crate::limits::{rate_limit, RateLimiter};

/// The explicit resource limits the MCP endpoint enforces (OWASP API4 —
/// Unrestricted Resource Consumption). The body-size limit bounds one request
/// (over it → `413`); the rate limit bounds the burst from one peer IP (over it →
/// `429`). A deployment tunes these from config; [`Default`] is a safe starting
/// point for a JSON-RPC control endpoint.
#[derive(Clone, Copy, Debug)]
pub struct ServeLimits {
    /// The largest request body accepted, in bytes. A larger body is rejected with
    /// `413 Payload Too Large` before it is buffered or dispatched to DORM.
    pub max_body_bytes: usize,
    /// How many requests one peer IP may make per [`window`](Self::rate_window)
    /// before `429 Too Many Requests`.
    pub max_requests_per_window: u32,
    /// The length of the fixed rate-limit window.
    pub rate_window: Duration,
}

impl ServeLimits {
    /// 256 KiB bodies: a JSON-RPC control line is tiny, so this is already generous
    /// and blunts an oversized-body resource attack.
    pub const DEFAULT_MAX_BODY_BYTES: usize = 256 * 1024;
    /// 300 requests per minute per peer IP — high enough not to trip a normal
    /// client, low enough to blunt a single abusive one.
    pub const DEFAULT_MAX_REQUESTS_PER_WINDOW: u32 = 300;
    /// The default rate-limit window (60 seconds).
    pub const DEFAULT_RATE_WINDOW: Duration = Duration::from_secs(60);
}

impl Default for ServeLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: Self::DEFAULT_MAX_BODY_BYTES,
            max_requests_per_window: Self::DEFAULT_MAX_REQUESTS_PER_WINDOW,
            rate_window: Self::DEFAULT_RATE_WINDOW,
        }
    }
}

/// Shared state the `/mcp` handler closes over: the base server configuration
/// (cloned per request, then narrowed to the principal), the authenticator, and
/// the resource [`ServeLimits`] the [`router`] enforces. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    cfg: Arc<ServerConfig>,
    auth: Arc<dyn Authenticator>,
    limits: ServeLimits,
}

impl AppState {
    /// Assemble the handler state from a base [`ServerConfig`] and an
    /// [`Authenticator`], with the default [`ServeLimits`]. `writable` on `cfg` is
    /// the server-wide ceiling: a request can only write if the server is writable
    /// *and* the principal may.
    pub fn new(cfg: ServerConfig, auth: Arc<dyn Authenticator>) -> Self {
        Self {
            cfg: Arc::new(cfg),
            auth,
            limits: ServeLimits::default(),
        }
    }

    /// Override the resource [`ServeLimits`] (body size, rate limit), builder-style.
    #[must_use]
    pub fn with_limits(mut self, limits: ServeLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// Build the axum [`Router`] for the MCP HTTP transport: `POST /mcp`, behind the
/// explicit resource limits in [`ServeLimits`] — a body-size cap (`413` over it)
/// and a per-peer-IP rate limit (`429` over it) — with the handler state attached.
/// Exposed so a host can mount it, add middleware, or drive it in-process for
/// tests without binding a socket.
pub fn router(state: AppState) -> Router {
    let limits = state.limits;
    let limiter = RateLimiter::new(limits.max_requests_per_window, limits.rate_window);
    Router::new()
        .route("/mcp", post(mcp_handler))
        // Innermost to outermost: the body cap rejects an oversized request (413)
        // before it is buffered, and the rate limit (outermost) sheds a burst
        // (429) before any work is done for it.
        .layer(DefaultBodyLimit::max(limits.max_body_bytes))
        .layer(from_fn_with_state(limiter, rate_limit))
        .with_state(state)
}

/// Pull the raw bearer token out of an `Authorization: Bearer <token>` header.
/// Returns `None` when the header is absent, not UTF-8, or not a bearer scheme.
fn bearer_of(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Map an [`AuthError`] to its HTTP response. A missing or invalid credential is
/// `401` (with a `WWW-Authenticate: Bearer` challenge); a forbidden one is `403`.
fn deny(err: AuthError) -> Response {
    match err {
        AuthError::MissingCredentials | AuthError::InvalidToken => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            err.to_string(),
        )
            .into_response(),
        AuthError::Forbidden => (StatusCode::FORBIDDEN, err.to_string()).into_response(),
    }
}

/// The `POST /mcp` handler: authenticate, then dispatch one JSON-RPC message
/// through DORM's [`dorm_mcp::handle_message`].
async fn mcp_handler(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // 1. Zero-trust gate — authenticate before anything else. The authenticator
    //    may be an OIDC/JWT verifier whose RSA/ECDSA signature check is CPU-bound
    //    and synchronous; run it on the blocking pool so a burst of verifications
    //    never stalls the async executor (and the DORM dispatch behind it).
    let Some(bearer) = bearer_of(&headers) else {
        return deny(AuthError::MissingCredentials);
    };
    let auth = Arc::clone(&state.auth);
    let principal = match tokio::task::spawn_blocking(move || auth.authenticate(&bearer)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return deny(e),
        // The verification task panicked — an internal fault, never an auth pass.
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "authentication task failed",
            )
                .into_response()
        }
    };

    // 2. Per-request config: the tenant and write scope come from the AUTHENTICATED
    //    principal, never from the request body (OWASP API1). Fail CLOSED on an
    //    untenanted caller — parity with the HTTP edge's `scope_of` (SE-4 / P0-4):
    //    a request with no tenant is DENIED, not served with `None`, which DORM
    //    reads as unscoped/global access across every tenant (the old fail-open).
    //    A single-tenant deployment may pin a base `cfg.tenant`; that is the only
    //    way a principal without its own tenant is served, and then only within
    //    that one pinned tenant — never globally.
    let Some(tenant) = principal
        .tenant
        .clone()
        .or_else(|| state.cfg.tenant.clone())
    else {
        return deny(AuthError::Forbidden);
    };
    let mut req_cfg = (*state.cfg).clone();
    req_cfg.tenant = Some(tenant);
    req_cfg.writable = state.cfg.writable && can_write(&principal);

    // 3. Body must be a UTF-8 JSON-RPC line; dispatch it to DORM's core.
    let Ok(line) = std::str::from_utf8(&body) else {
        return (StatusCode::BAD_REQUEST, "request body is not valid UTF-8").into_response();
    };

    match dorm_mcp::handle_message(&req_cfg, line).await {
        // A request with a response.
        Some(json) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response(),
        // A notification (or an unparsable line JSON-RPC says to ignore): no body.
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Serve the MCP HTTP transport on `addr` until Ctrl-C, with Seiva auth in front
/// of DORM's protocol core. A convenience wrapper over
/// [`serve_with_shutdown`] using Ctrl-C as the shutdown signal.
pub async fn serve(
    addr: std::net::SocketAddr,
    cfg: ServerConfig,
    auth: Arc<dyn Authenticator>,
) -> std::io::Result<()> {
    serve_with_shutdown(addr, cfg, auth, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

/// Serve the MCP HTTP transport on `addr`, shutting down gracefully when
/// `shutdown` resolves. Binds a listener, mounts [`router`], and runs it with
/// axum's graceful-shutdown hook so in-flight requests drain before exit.
pub async fn serve_with_shutdown<S>(
    addr: std::net::SocketAddr,
    cfg: ServerConfig,
    auth: Arc<dyn Authenticator>,
    shutdown: S,
) -> std::io::Result<()>
where
    S: std::future::Future<Output = ()> + Send + 'static,
{
    let state = AppState::new(cfg, auth);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // `into_make_service_with_connect_info` installs the peer `SocketAddr` so the
    // rate limiter keys each client independently rather than collapsing them all
    // into the one fallback bucket.
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Principal, TokenAuthenticator, WRITE_PERMISSION};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    /// A [`Principal`] in the shared identity model: `can_write` is expressed as
    /// the presence of [`WRITE_PERMISSION`], not a struct field.
    fn principal(tenant: Option<&str>, can_write: bool) -> Principal {
        let permissions: Vec<String> = can_write
            .then(|| WRITE_PERMISSION.to_string())
            .into_iter()
            .collect();
        match tenant {
            Some(t) => Principal::in_tenant(t, t, permissions),
            None => Principal::new("root", permissions),
        }
    }

    fn state_with(auth: TokenAuthenticator) -> AppState {
        AppState::new(ServerConfig::default(), Arc::new(auth))
    }

    fn post_mcp(bearer: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder().method("POST").uri("/mcp");
        if let Some(t) = bearer {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    #[tokio::test]
    async fn no_bearer_is_rejected_401() {
        let app = router(state_with(TokenAuthenticator::new()));
        let resp = app
            .oneshot(post_mcp(
                None,
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // The zero-trust gate advertises the scheme it wants.
        assert_eq!(
            resp.headers().get(header::WWW_AUTHENTICATE).unwrap(),
            "Bearer"
        );
    }

    #[tokio::test]
    async fn an_unknown_token_is_rejected_401() {
        let app = router(state_with(
            TokenAuthenticator::new().with_token("good", principal(Some("acme"), false)),
        ));
        let resp = app
            .oneshot(post_mcp(
                Some("bad"),
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_valid_token_reaches_the_protocol_core() {
        // `initialize` is answered by the protocol core without touching the DB,
        // so this exercises the whole auth + dispatch path end to end.
        let app = router(state_with(
            TokenAuthenticator::new().with_token("good", principal(Some("acme"), false)),
        ));
        let resp = app
            .oneshot(post_mcp(
                Some("good"),
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["id"], serde_json::json!(1));
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    }

    #[tokio::test]
    async fn an_untenanted_caller_is_denied_not_served_globally() {
        // SE-4 / P0-4: a principal with no tenant must be REFUSED (fail-closed),
        // mirroring the HTTP edge — never served with `None`, which DORM reads as
        // unscoped access across every tenant. `ServerConfig::default()` pins no
        // base tenant, so there is no fallback and the request is forbidden.
        let app = router(state_with(
            TokenAuthenticator::new().with_token("notenant", principal(None, false)),
        ));
        let resp = app
            .oneshot(post_mcp(
                Some("notenant"),
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "an untenanted MCP caller must be denied, not served globally"
        );
    }

    #[tokio::test]
    async fn a_notification_from_a_valid_token_is_204() {
        // A JSON-RPC notification (no `id`) yields no response; the handler maps
        // DORM's `None` to 204 — authenticated, dispatched, no DB touched.
        let app = router(state_with(
            TokenAuthenticator::new().with_token("good", principal(Some("acme"), false)),
        ));
        let resp = app
            .oneshot(post_mcp(
                Some("good"),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_rejected_413() {
        // OWASP API4: a body larger than the configured ceiling is refused with
        // `413 Payload Too Large` before it is buffered or dispatched to DORM.
        let limits = ServeLimits {
            max_body_bytes: 64,
            ..ServeLimits::default()
        };
        let state = AppState::new(ServerConfig::default(), Arc::new(TokenAuthenticator::new()))
            .with_limits(limits);
        let app = router(state);

        // A ~1 KiB body, well over the 64-byte cap. (Rejected on size before auth.)
        let oversized = format!(
            r#"{{"jsonrpc":"2.0","id":1,"padding":"{}"}}"#,
            "x".repeat(1024)
        );
        let resp = app
            .oneshot(post_mcp(Some("anything"), &oversized))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_burst_over_the_rate_limit_is_rejected_429() {
        // OWASP API4: with a tiny budget, the request past it is shed with `429`.
        // The test harness drives the router without `ConnectInfo`, so every
        // request shares the one fallback bucket — enough to trip the limit.
        let limits = ServeLimits {
            max_requests_per_window: 2,
            rate_window: Duration::from_secs(60),
            ..ServeLimits::default()
        };
        let state = AppState::new(
            ServerConfig::default(),
            Arc::new(TokenAuthenticator::new().with_token("good", principal(Some("acme"), false))),
        )
        .with_limits(limits);
        let app = router(state);

        let hit = || {
            let app = app.clone();
            async move {
                app.oneshot(post_mcp(
                    Some("good"),
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
                ))
                .await
                .unwrap()
                .status()
            }
        };

        // Two within budget, the third shed.
        assert_eq!(hit().await, StatusCode::OK);
        assert_eq!(hit().await, StatusCode::OK);
        assert_eq!(hit().await, StatusCode::TOO_MANY_REQUESTS);
    }
}
