//! The MCP HTTP transport Seiva owns, in front of DORM's protocol core.
//!
//! One endpoint, `POST /mcp`, that in order:
//! 1. **authenticates first** — extracts the `Authorization: Bearer <token>`
//!    header and runs it through the [`Authenticator`]; a missing or invalid
//!    credential is rejected (`401`) and a forbidden one (`403`) before the body
//!    is read or DORM is touched. This is the zero-trust gate — deny by default.
//! 2. builds a **per-request** [`dorm_mcp::ServerConfig`] from the base config,
//!    overriding `tenant` with the principal's tenant and narrowing `writable` to
//!    `base.writable && principal.can_write`, so the tenant and write scope come
//!    from the authenticated principal, never from a tool argument;
//! 3. hands the raw JSON-RPC request line to [`dorm_mcp::handle_message`] — the
//!    protocol core DORM owns — and returns its response as `application/json`
//!    (`200`), or `204 No Content` when it is a notification (no response).
//!
//! DORM owns the MCP protocol (`handle_message`) and the `stdio` transport; this
//! module is purely the HTTP skin plus Seiva's auth. It does not reimplement any
//! method dispatch, and it does not call DORM's own `serve_http` / `serve_on`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use dorm_mcp::ServerConfig;

use crate::auth::{AuthError, Authenticator};

/// Shared state the `/mcp` handler closes over: the base server configuration
/// (cloned per request, then narrowed to the principal) and the authenticator.
/// Cheap to clone — both fields are behind an [`Arc`].
#[derive(Clone)]
pub struct AppState {
    cfg: Arc<ServerConfig>,
    auth: Arc<dyn Authenticator>,
}

impl AppState {
    /// Assemble the handler state from a base [`ServerConfig`] and an
    /// [`Authenticator`]. `writable` on `cfg` is the server-wide ceiling: a
    /// request can only write if the server is writable *and* the principal may.
    pub fn new(cfg: ServerConfig, auth: Arc<dyn Authenticator>) -> Self {
        Self {
            cfg: Arc::new(cfg),
            auth,
        }
    }
}

/// Build the axum [`Router`] for the MCP HTTP transport: `POST /mcp`, with the
/// handler state attached. Exposed so a host can mount it, add middleware, or
/// drive it in-process for tests without binding a socket.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/mcp", post(mcp_handler))
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
    // 1. Zero-trust gate — authenticate before anything else.
    let Some(bearer) = bearer_of(&headers) else {
        return deny(AuthError::MissingCredentials);
    };
    let principal = match state.auth.authenticate(&bearer) {
        Ok(p) => p,
        Err(e) => return deny(e),
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
    req_cfg.writable = state.cfg.writable && principal.can_write;

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
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Principal, TokenAuthenticator};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

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
        let app = router(state_with(TokenAuthenticator::new().with_token(
            "good",
            Principal {
                subject: "acme".into(),
                tenant: Some("acme".into()),
                can_write: false,
            },
        )));
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
        let app = router(state_with(TokenAuthenticator::new().with_token(
            "good",
            Principal {
                subject: "acme".into(),
                tenant: Some("acme".into()),
                can_write: false,
            },
        )));
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
        let app = router(state_with(TokenAuthenticator::new().with_token(
            "notenant",
            Principal {
                subject: "root".into(),
                tenant: None,
                can_write: false,
            },
        )));
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
        let app = router(state_with(TokenAuthenticator::new().with_token(
            "good",
            Principal {
                subject: "acme".into(),
                tenant: Some("acme".into()),
                can_write: false,
            },
        )));
        let resp = app
            .oneshot(post_mcp(
                Some("good"),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
