//! Delonix Seiva — the service runtime ("the main a module installs").
//!
//! This crate is the runtime that boots a Seiva service. It is the piece a
//! module's binary installs instead of writing its own `main`; it replaces
//! DORM's old `dorm-api`. It does four things, each delegated to a focused
//! framework crate so nothing is re-implemented here:
//!
//! - **Mounts the HTTP application** — the facade-driven controllers, the
//!   `/health/{live,ready}` endpoints and a `/openapi.json` composed from **every**
//!   mounted facade. It composes the pieces [`seiva_http::router`] offers (its
//!   `notes` and health routers, the body-limit and request-id layers) with the
//!   `accounts` module's controller router ([`accounts_interface::router`]) so a
//!   request to an accounts route hits the real module use case — and serves its
//!   own broader [`openapi_document`] rather than inheriting `seiva-http`'s
//!   notes-only one. See [`serve`].
//! - **Runs a background worker host** on the framework's worker runtime
//!   ([`delonix_seiva::worker::run_worker`]). See [`run_workers`].
//! - **Reads 12-factor configuration** from the environment with [`seiva_config`],
//!   validated at startup and guarded against dev shortcuts in production. See
//!   [`ServerConfig`].
//! - **Shuts down gracefully**: on a shutdown signal it flips the process to
//!   [`HealthState::Draining`](seiva_http::HealthState) — so a load balancer takes
//!   it out of rotation — and drains in-flight requests before exiting.
//!
//! It is a LIBRARY (so each piece is testable without a live port or a running
//! clock) plus a thin binary (`seiva-server`) that wires the pieces together.
//!
//! # DORM-free
//!
//! This crate depends on no `dorm-*` crate and on none of the excluded,
//! DORM-git-coupled Seiva crates (`seiva-data-dorm`, `seiva-outbox-relay`,
//! `seiva-rag-worker`, `seiva-mcp`). It declares no cargo feature that could pull
//! any of them in. [`run_workers`] therefore ships a no-op demonstration pass
//! today; the real transactional-outbox relay pass is injected here behind the
//! framework's `dorm` feature once the CI token for the private DORM repo exists.
//!
//! # Example
//!
//! ```no_run
//! use seiva_server::{run_workers, serve, ServerConfig};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = ServerConfig::from_process_env()?;
//! // A resolved shutdown future would stop both at once; a real binary feeds a
//! // Ctrl-C / SIGTERM future to each (see `src/main.rs`).
//! let (served, _reports) = tokio::join!(serve(config, async {}), run_workers(async {}));
//! served?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::fmt;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn;
use axum::routing::get;
use axum::{Json, Router};
use delonix_seiva::ports::PortResult;
use delonix_seiva::worker::{run_worker, WorkerConfig};
use seiva_config::{EnvReader, FromEnv};
use seiva_http::{health_router, notes_router, request_id, Health, DEFAULT_BODY_LIMIT_BYTES};
use tokio::net::TcpListener;

use accounts_facade::AccountsFacade;
use delonix_seiva::contract::Facade;
use notes_facade::NotesFacade;

pub use delonix_seiva::worker::WorkerReport;
pub use seiva_config::{AppEnv, ConfigError};

/// The environment variable holding the full HTTP bind address (`host:port`).
/// Highest precedence; see [`ServerConfig`].
pub const HTTP_ADDR_VAR: &str = "SEIVA_HTTP_ADDR";

/// The environment variable holding a bare port, the convention many hosting
/// platforms (Heroku, Cloud Run, …) set. Used only when [`HTTP_ADDR_VAR`] is
/// unset; see [`ServerConfig`].
pub const PORT_VAR: &str = "PORT";

/// The environment variable for the development-only "expose internal error
/// details to clients" shortcut. Guarded: it must not be enabled in production.
pub const EXPOSE_ERROR_DETAILS_VAR: &str = "SEIVA_EXPOSE_ERROR_DETAILS";

/// The default HTTP bind address when neither [`HTTP_ADDR_VAR`] nor [`PORT_VAR`]
/// is set: loopback only, so an unconfigured service is not exposed off-box.
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8080";

/// The runtime configuration of a Seiva service, read from the environment.
///
/// It is small but real. Build it with [`from_process_env`](ServerConfig::from_process_env)
/// (the 12-factor path) or, in tests, with [`seiva_config::load_from`] over an
/// explicit map (deterministic, never racing on the process-global environment).
///
/// # HTTP address precedence
///
/// The bind address is resolved in this order, highest first:
///
/// 1. [`SEIVA_HTTP_ADDR`](HTTP_ADDR_VAR) — a full `host:port` (e.g.
///    `0.0.0.0:8080`). Wins whenever it is set.
/// 2. [`PORT`](PORT_VAR) — a bare port, the convention hosting platforms set.
///    Bound on `0.0.0.0` (all interfaces), since a platform that sets `PORT`
///    routes external traffic to the container.
/// 3. [`DEFAULT_HTTP_ADDR`] — `127.0.0.1:8080`, loopback only.
///
/// # Production-safety guard
///
/// [`EXPOSE_ERROR_DETAILS_VAR`] is a development convenience. Through
/// [`AppEnv::reject_dev_shortcut`] it is a *startup error* to enable it when
/// `APP_ENV=production`, so the mistake fails the boot rather than leaking
/// internals on the first error in production.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The runtime environment (`APP_ENV`), defaulting to
    /// [`Development`](AppEnv::Development).
    pub env: AppEnv,
    /// The resolved HTTP bind address (see the precedence on [`ServerConfig`]).
    pub http_addr: SocketAddr,
    /// Whether internal error details are exposed to clients. A development-only
    /// shortcut, refused in production by the guard.
    pub expose_error_details: bool,
}

impl ServerConfig {
    /// Loads and validates the configuration from the live process environment.
    ///
    /// This is what a `main` calls: on success it holds a fully typed, fully
    /// validated configuration; on failure every problem is reported together.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when `APP_ENV` is unrecognised, when an address
    /// variable fails to parse, or when the production guard rejects a dev
    /// shortcut.
    pub fn from_process_env() -> Result<Self, ConfigError> {
        seiva_config::load::<Self>()
    }

    /// The [`DEFAULT_HTTP_ADDR`] parsed into a [`SocketAddr`].
    #[must_use]
    pub fn default_http_addr() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 8080))
    }
}

/// Resolves the HTTP bind address from the reader, honouring the precedence
/// documented on [`ServerConfig`]. A present-but-unparseable value records a
/// problem (and yields `None`) so a typo fails the boot.
fn resolve_http_addr(reader: &mut EnvReader) -> Option<SocketAddr> {
    if reader.optional_var(HTTP_ADDR_VAR).is_some() {
        reader.optional_parsed::<SocketAddr>(HTTP_ADDR_VAR, ServerConfig::default_http_addr())
    } else if reader.optional_var(PORT_VAR).is_some() {
        let port = reader.optional_parsed::<u16>(PORT_VAR, 0)?;
        Some(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
    } else {
        Some(ServerConfig::default_http_addr())
    }
}

impl FromEnv for ServerConfig {
    fn from_env(reader: &mut EnvReader) -> Option<Self> {
        // Read every field up front so each problem is collected, not just the
        // first; the `?`s below only decide whether to assemble.
        let env = reader.app_env();
        let http_addr = resolve_http_addr(reader);
        let expose_error_details = reader.optional_bool(EXPOSE_ERROR_DETAILS_VAR, false);

        let config = Self {
            env,
            http_addr: http_addr?,
            expose_error_details: expose_error_details?,
        };

        reader.guard(
            config
                .env
                .reject_dev_shortcut(EXPOSE_ERROR_DETAILS_VAR, config.expose_error_details),
        );

        Some(config)
    }
}

/// An error raised while binding or serving the HTTP application.
///
/// It wraps the pointer-sized [`std::io::Error`] from the failing stage, so it is
/// cheap to return by value (it does not trip clippy's `result_large_err`).
#[derive(Debug)]
#[non_exhaustive]
pub enum ServeError {
    /// Binding the [`TcpListener`] to the configured address failed (the address
    /// is in use, not permitted, or unavailable).
    Bind(std::io::Error),
    /// The server itself failed while accepting or draining connections.
    Serve(std::io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::Bind(e) => write!(f, "failed to bind the HTTP listener: {e}"),
            ServeError::Serve(e) => write!(f, "the HTTP server failed: {e}"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServeError::Bind(e) | ServeError::Serve(e) => Some(e),
        }
    }
}

/// The service's OpenAPI 3.1 document, composed from **both** mounted facades.
///
/// [`seiva_http::openapi_document`] only knows the facade `seiva-http` mounts
/// (`Notes`), so its document never covers `accounts`. This crate owns the full
/// mounted surface, so it composes its own document from
/// `[NotesFacade::DESCRIPTOR, AccountsFacade::DESCRIPTOR]` through the same
/// generator ([`seiva_openapi::openapi_document`]). Adding a facade to the
/// application is therefore one more descriptor here — the paths, operations,
/// permissions and schemas all follow from the descriptors.
#[must_use]
pub fn openapi_document() -> serde_json::Value {
    seiva_openapi::openapi_document(
        seiva_openapi::ApiInfo {
            title: "Delonix Seiva service".to_string(),
            version: delonix_seiva::VERSION.to_string(),
        },
        &[NotesFacade::DESCRIPTOR, AccountsFacade::DESCRIPTOR],
    )
}

/// `GET /openapi.json` — serves the [`openapi_document`] composed from every
/// mounted facade.
async fn openapi_handler() -> Json<serde_json::Value> {
    Json(openapi_document())
}

/// Builds the full HTTP application: the demo `notes` controller, the health
/// endpoints, the `accounts` module's controller router
/// ([`accounts_interface::router`]) and the composed `/openapi.json`, wrapped in
/// the body-size limit and request-id layers.
///
/// `/openapi.json` is served **here**, from [`openapi_document`] (which covers
/// both `notes` and `accounts`), rather than inherited from
/// [`seiva_http::router`]. `seiva_http::router` registers its own, notes-only
/// `/openapi.json`; mounting that alongside this crate's route would register the
/// same path twice and panic axum at startup. So this crate composes the pieces
/// `seiva_http::router` would (its [`notes_router`] and [`health_router`], the
/// same [`DEFAULT_BODY_LIMIT_BYTES`] and [`request_id`](fn@seiva_http::request_id)
/// layers — `request_id` names both a module and the middleware function in
/// `seiva_http`, so the link disambiguates to the function) and serves the broader
/// document itself — never routing through `seiva_http::router`.
///
/// The accounts routes live on distinct paths (`/v1/accounts/...`), so the merge
/// adds them without conflict, and a request to one hits the real module use case
/// over its in-memory service. Because the layers wrap the merged router, the
/// accounts surface now sits inside the body-limit and request-id middleware too.
fn application(health: Health) -> Router {
    Router::new()
        .merge(notes_router())
        .merge(health_router(health))
        .merge(accounts_interface::router())
        .route("/openapi.json", get(openapi_handler))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES))
        .layer(from_fn(request_id))
}

/// Serves the Seiva HTTP application until `shutdown` resolves, then drains.
///
/// The lifecycle, in order:
///
/// 1. Builds the application (the framework router merged with the `accounts`
///    controller router; see the private `application` builder) over a fresh
///    [`Health`] handle in the
///    [`Starting`](seiva_http::HealthState::Starting) state, so
///    `/health/ready` answers `503` until the socket is live.
/// 2. Binds a [`TcpListener`] to [`ServerConfig::http_addr`]; a bind failure is
///    returned as [`ServeError::Bind`].
/// 3. Flips `Health` [`Starting -> Ready`](seiva_http::Health::set_ready) now the
///    socket is listening, so load balancers may route traffic.
/// 4. Serves with [`axum::serve()`] and a graceful-shutdown future that, once
///    `shutdown` resolves, flips `Health` to
///    [`Draining`](seiva_http::Health::set_draining) *before* the server stops
///    accepting — so `/health/ready` returns `503` and the process leaves
///    rotation while in-flight requests finish.
///
/// # Errors
///
/// Returns [`ServeError::Bind`] if the listener cannot bind, or
/// [`ServeError::Serve`] if serving fails.
pub async fn serve(
    config: ServerConfig,
    // `'static` is required by `axum::serve(..).with_graceful_shutdown(..)`: the
    // graceful-shutdown future is held by the server task for its whole life.
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let health = Health::starting();
    let app = application(health.clone());

    let listener = TcpListener::bind(config.http_addr)
        .await
        .map_err(ServeError::Bind)?;

    // The socket is live: Starting -> Ready, so /health/ready passes.
    health.set_ready();

    let draining = {
        let health = health.clone();
        async move {
            shutdown.await;
            // Draining BEFORE the server stops accepting: /health/ready goes 503
            // and a load balancer takes us out of rotation while we drain.
            health.set_draining();
        }
    };

    // `into_make_service_with_connect_info` installs the peer `SocketAddr` as a
    // request extension, so the rate limiter in `seiva_http` can key on the
    // client IP rather than a shared fallback bucket.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(draining)
    .await
    .map_err(ServeError::Serve)?;

    Ok(())
}

/// The default interval between background worker passes. A pass that drains a
/// backlog could poll faster; the demonstration pass has no backlog, so it idles
/// at this cadence until shutdown.
pub const DEFAULT_WORKER_INTERVAL: Duration = Duration::from_secs(30);

/// The demonstration worker pass: a no-op that processes nothing and never fails.
///
/// It exists so the worker host has a real pass to drive today, DORM-free. The
/// real transactional-outbox relay pass (`seiva-outbox-relay`, DORM-git-coupled)
/// is injected in its place behind the framework's `dorm` feature once the
/// private-repo CI token exists — this crate does not wire DORM now.
///
/// # Errors
///
/// Never errors; the [`PortResult`] return type matches the shape
/// [`run_worker`] drives.
pub async fn demo_pass() -> PortResult<u64> {
    Ok(0)
}

/// Runs the background worker host until `shutdown` resolves, returning one
/// [`WorkerReport`] per worker.
///
/// Today it drives a single worker over [`demo_pass`] through
/// [`delonix_seiva::worker::run_worker`], which owns the loop, the interval, the
/// shutdown check and the metrics. A resolved `shutdown` ends the host within one
/// pass.
///
/// To add real passes (e.g. the DORM outbox relay once the `dorm` feature is
/// wired), fan a single shutdown out to each worker over a `watch` channel and
/// collect their reports — the host shape is otherwise unchanged.
pub async fn run_workers(shutdown: impl Future<Output = ()> + Send) -> Vec<WorkerReport> {
    let config = WorkerConfig::new("demo-noop", DEFAULT_WORKER_INTERVAL);
    let report = run_worker(config, shutdown, demo_pass).await;
    vec![report]
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use seiva_config::load_from;
    use std::collections::HashMap;
    use tower::ServiceExt; // for `oneshot`

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    // --- Config: parsing from an explicit map (no env races) ----------------

    #[test]
    fn config_defaults_to_loopback_8080_and_development() {
        let cfg: ServerConfig = load_from(map(&[])).expect("empty env is a valid dev config");
        assert_eq!(cfg.http_addr, ServerConfig::default_http_addr());
        assert_eq!(cfg.http_addr.to_string(), "127.0.0.1:8080");
        assert!(cfg.env.is_development());
        assert!(!cfg.expose_error_details);
    }

    #[test]
    fn config_http_addr_var_wins_over_port() {
        // Both set: SEIVA_HTTP_ADDR takes precedence.
        let cfg: ServerConfig =
            load_from(map(&[(HTTP_ADDR_VAR, "0.0.0.0:9000"), (PORT_VAR, "1234")]))
                .expect("valid address");
        assert_eq!(cfg.http_addr.to_string(), "0.0.0.0:9000");
    }

    #[test]
    fn config_port_fallback_binds_all_interfaces() {
        let cfg: ServerConfig = load_from(map(&[(PORT_VAR, "8081")])).expect("valid port fallback");
        assert_eq!(cfg.http_addr.to_string(), "0.0.0.0:8081");
    }

    #[test]
    fn config_rejects_unparseable_address() {
        let err = load_from::<ServerConfig>(map(&[(HTTP_ADDR_VAR, "not-an-addr")])).unwrap_err();
        assert!(err.problems()[0].contains(HTTP_ADDR_VAR));
    }

    #[test]
    fn config_production_guard_rejects_expose_error_details() {
        let err = load_from::<ServerConfig>(map(&[
            ("APP_ENV", "production"),
            (EXPOSE_ERROR_DETAILS_VAR, "true"),
        ]))
        .unwrap_err();
        assert_eq!(err.problems().len(), 1);
        assert!(err.problems()[0].contains(EXPOSE_ERROR_DETAILS_VAR));
        assert!(err.problems()[0].contains("production"));
    }

    #[test]
    fn config_development_allows_the_same_shortcut() {
        let cfg: ServerConfig = load_from(map(&[
            ("APP_ENV", "development"),
            (EXPOSE_ERROR_DETAILS_VAR, "true"),
        ]))
        .expect("dev permits the shortcut");
        assert!(cfg.expose_error_details);
        assert!(cfg.env.is_development());
    }

    // --- HTTP surface: driven via the router, no live port -----------------

    #[tokio::test]
    async fn health_live_is_ok_and_ready_tracks_the_lifecycle() {
        let health = Health::starting();
        let app = seiva_http::router(health.clone());

        // /health/live is 200 in every state.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // /health/ready is 503 while Starting...
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);

        // ...and 200 once Ready (the clone shares the atomic serve() flips).
        health.set_ready();
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
    }

    // --- accounts module: the merged app serves the real use case ----------

    /// The merged application serves the `accounts` create route: an anonymous
    /// caller is denied with an RFC 9457 `403`, and a caller holding
    /// `accounts.create` with a valid body succeeds (`200`, the generic write
    /// branch) with a secret-free user. This proves `serve` mounts the real
    /// module, not just the demo.
    #[tokio::test]
    async fn merged_app_serves_the_accounts_create_route() {
        let app = application(Health::starting());

        // Without the permission: 403 problem.
        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/accounts/create_user")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        // With the permission and a valid body: 200 and a secret-free user.
        let ok = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/accounts/create_user")
                    .header("content-type", "application/json")
                    .extension(seiva_http::Principal::new("user", ["accounts.create"]))
                    .body(Body::from(r#"{"username":"alice","password":"hunter2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(ok.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["username"], "alice");
        let rendered = body.to_string();
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("$argon2id$"),
            "no secret must appear in the response: {rendered}"
        );
    }

    // --- facade discovery: /openapi.json lists every mounted module ---------

    /// The served `/openapi.json` documents BOTH the demonstration `Notes` facade
    /// and the mounted `accounts` module, proving link-time discovery
    /// (`registered_facades`) picks up every `#[seiva::facade]` in the binary
    /// with no hand-maintained list (OWASP API9).
    #[tokio::test]
    async fn openapi_discovers_every_mounted_facade() {
        let app = application(Health::starting());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let paths = doc["paths"].as_object().expect("paths object");
        assert!(
            paths.keys().any(|p| p.contains("/notes")),
            "the Notes demonstration facade must be documented: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
        assert!(
            paths.keys().any(|p| p.contains("/accounts")),
            "the mounted accounts module must be discovered and documented: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }

    // --- serve: binds an ephemeral port, drains on a resolved shutdown ------

    #[tokio::test]
    async fn serve_binds_ephemeral_and_returns_on_resolved_shutdown() {
        let config = ServerConfig {
            env: AppEnv::Development,
            // Port 0: the OS assigns a free ephemeral port, so the test never
            // races another on a fixed port.
            http_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            expose_error_details: false,
        };
        // An already-resolved shutdown: serve binds, flips Ready, immediately
        // drains and returns Ok — no fixed port, no running clock.
        serve(config, async {})
            .await
            .expect("serve binds and drains cleanly");
    }

    // --- worker host: stops on a resolved shutdown, reports zero processed --

    #[tokio::test]
    async fn run_workers_stops_and_reports_the_demo_pass() {
        let reports = run_workers(async {}).await;
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert!(report.passes >= 1, "at least one pass runs before shutdown");
        assert_eq!(report.processed, 0, "the demo pass processes nothing");
        assert_eq!(report.errors, 0, "the demo pass never fails");
    }
}
