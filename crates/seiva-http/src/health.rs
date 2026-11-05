//! Liveness and readiness endpoints.
//!
//! [`Health`] is a cheap, cloneable handle over a shared lifecycle state that
//! moves `Starting -> Ready -> Draining`. [`health_router`] mounts
//! `/health/live` and `/health/ready`:
//!
//! - **`/health/live`** reports that the process is up. It returns `200` in every
//!   state (including `Draining`): a live-but-draining process must not be killed
//!   mid-shutdown, only taken out of rotation.
//! - **`/health/ready`** reports whether the process should receive traffic. It
//!   returns `200` only while [`HealthState::Ready`]; `Starting` and `Draining`
//!   return `503` so a load balancer stops routing to it.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

/// The lifecycle state of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Still warming up (loading config, opening pools). Not ready for traffic.
    Starting,
    /// Serving traffic.
    Ready,
    /// Shutting down gracefully: still alive, but should receive no new traffic.
    Draining,
}

impl HealthState {
    /// The lowercase token used in the JSON body.
    pub fn as_str(self) -> &'static str {
        match self {
            HealthState::Starting => "starting",
            HealthState::Ready => "ready",
            HealthState::Draining => "draining",
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => HealthState::Ready,
            2 => HealthState::Draining,
            _ => HealthState::Starting,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            HealthState::Starting => 0,
            HealthState::Ready => 1,
            HealthState::Draining => 2,
        }
    }
}

/// A cloneable handle over the shared process lifecycle state. Clones share one
/// atomic, so a shutdown hook can flip the state the router observes.
#[derive(Debug, Clone)]
pub struct Health {
    state: Arc<AtomicU8>,
}

impl Health {
    /// A new handle in the [`HealthState::Starting`] state.
    pub fn starting() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(HealthState::Starting.as_u8())),
        }
    }

    /// The current state.
    pub fn state(&self) -> HealthState {
        HealthState::from_u8(self.state.load(Ordering::SeqCst))
    }

    /// Sets the state explicitly.
    pub fn set(&self, state: HealthState) {
        self.state.store(state.as_u8(), Ordering::SeqCst);
    }

    /// Marks the process ready for traffic.
    pub fn set_ready(&self) {
        self.set(HealthState::Ready);
    }

    /// Marks the process draining (graceful shutdown).
    pub fn set_draining(&self) {
        self.set(HealthState::Draining);
    }

    /// Whether the process is currently ready for traffic.
    pub fn is_ready(&self) -> bool {
        self.state() == HealthState::Ready
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::starting()
    }
}

/// A critical-dependency readiness check (e.g. "can the service reach its
/// database?"). The composition root supplies one as a closure returning a boxed
/// future; readiness runs it behind a short cache so a probe flood cannot hammer
/// the dependency (§8).
///
/// ```
/// use std::sync::Arc;
/// use seiva_http::ReadinessCheck;
/// let check: ReadinessCheck = Arc::new(|| Box::pin(async { true }));
/// ```
pub type ReadinessCheck = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// A readiness dependency with a cached result, so the probe runs at most once
/// per `ttl` no matter how often `/health/ready` is polled (bounded load, §8).
#[derive(Clone)]
struct ReadinessDep {
    check: ReadinessCheck,
    ttl: Duration,
    cache: Arc<Mutex<Option<(Instant, bool)>>>,
}

impl ReadinessDep {
    /// The dependency's health, from the cache when fresh, else by running the
    /// check once and caching it. The lock is never held across the `await`.
    async fn healthy(&self) -> bool {
        if let Some((at, ok)) = *self.cache.lock().unwrap_or_else(|e| e.into_inner()) {
            if at.elapsed() < self.ttl {
                return ok;
            }
        }
        let ok = (self.check)().await;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), ok));
        ok
    }
}

/// The state the readiness route carries: the lifecycle handle and an optional
/// cached dependency check.
#[derive(Clone)]
struct ReadyState {
    health: Health,
    dep: Option<ReadinessDep>,
}

/// The JSON body of a health response.
#[derive(Debug, Serialize)]
struct HealthBody {
    status: &'static str,
    state: &'static str,
}

/// `GET /health/live` — alive in every state, so a draining process is not
/// killed mid-shutdown. It **never** consults a dependency: a transient database
/// blip must not make the orchestrator restart a healthy process (§8).
async fn live(State(health): State<Health>) -> (StatusCode, Json<HealthBody>) {
    (
        StatusCode::OK,
        Json(HealthBody {
            status: "live",
            state: health.state().as_str(),
        }),
    )
}

/// `GET /health/ready` — `200` only while [`HealthState::Ready`] **and** the
/// optional dependency check (cached) passes; otherwise `503` so a load balancer
/// stops routing to it.
async fn ready(State(state): State<ReadyState>) -> (StatusCode, Json<HealthBody>) {
    let lifecycle = state.health.state();
    if lifecycle != HealthState::Ready {
        return unready(lifecycle);
    }
    match &state.dep {
        Some(dep) if !dep.healthy().await => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthBody {
                status: "unready",
                state: "dependency-unready",
            }),
        ),
        _ => (
            StatusCode::OK,
            Json(HealthBody {
                status: "ready",
                state: lifecycle.as_str(),
            }),
        ),
    }
}

fn unready(state: HealthState) -> (StatusCode, Json<HealthBody>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(HealthBody {
            status: "unready",
            state: state.as_str(),
        }),
    )
}

/// Builds the health sub-router backed by the given [`Health`] handle, with no
/// dependency check: readiness tracks the lifecycle state only.
pub fn health_router(health: Health) -> Router {
    build_router(health, None)
}

/// Builds the health sub-router with a cached critical-dependency readiness
/// check: `/health/ready` is `200` only while `Ready` **and** `check` passes,
/// and `check` runs at most once per `ttl` however often it is polled. Liveness
/// ignores the dependency entirely (§8).
pub fn health_router_with_dependency(
    health: Health,
    check: ReadinessCheck,
    ttl: Duration,
) -> Router {
    build_router(
        health,
        Some(ReadinessDep {
            check,
            ttl,
            cache: Arc::new(Mutex::new(None)),
        }),
    )
}

fn build_router(health: Health, dep: Option<ReadinessDep>) -> Router {
    // `live` keys off the lifecycle handle only; `ready` also needs the optional
    // dependency, so the two routes carry different state and are merged after
    // each is fully stated.
    let live_route = Router::new()
        .route("/health/live", get(live))
        .with_state(health.clone());
    let ready_route = Router::new()
        .route("/health/ready", get(ready))
        .with_state(ReadyState { health, dep });
    live_route.merge(ready_route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tower::ServiceExt;

    async fn status(router: &Router, path: &str) -> StatusCode {
        router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn readiness_reflects_the_dependency_but_liveness_ignores_it() {
        let health = Health::starting();
        health.set_ready();
        let healthy = Arc::new(AtomicBool::new(true));
        let flag = healthy.clone();
        // TTL zero: every readiness check consults the dependency (no caching),
        // so the test sees the flag flip immediately.
        let check: ReadinessCheck = Arc::new(move || {
            let flag = flag.clone();
            Box::pin(async move { flag.load(Ordering::SeqCst) })
        });
        let app = health_router_with_dependency(health, check, Duration::ZERO);

        // Ready + dependency healthy → 200.
        assert_eq!(status(&app, "/health/ready").await, StatusCode::OK);

        // Dependency goes down → readiness 503, but liveness stays 200 (no
        // restart storm from a transient dependency blip).
        healthy.store(false, Ordering::SeqCst);
        assert_eq!(
            status(&app, "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(status(&app, "/health/live").await, StatusCode::OK);

        // Dependency recovers → readiness 200 again.
        healthy.store(true, Ordering::SeqCst);
        assert_eq!(status(&app, "/health/ready").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_dependency_check_is_cached_within_its_ttl() {
        let health = Health::starting();
        health.set_ready();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let check: ReadinessCheck = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { true })
        });
        // A long TTL: repeated polls must consult the dependency at most once.
        let app = health_router_with_dependency(health, check, Duration::from_secs(300));

        for _ in 0..5 {
            assert_eq!(status(&app, "/health/ready").await, StatusCode::OK);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the dependency is probed once within the TTL, not per request"
        );
    }

    #[tokio::test]
    async fn readiness_without_a_dependency_tracks_only_the_lifecycle() {
        let health = Health::starting();
        let app = health_router(health.clone());
        assert_eq!(
            status(&app, "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        health.set_ready();
        assert_eq!(status(&app, "/health/ready").await, StatusCode::OK);
        health.set_draining();
        assert_eq!(
            status(&app, "/health/ready").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        // Liveness is 200 throughout.
        assert_eq!(status(&app, "/health/live").await, StatusCode::OK);
    }
}
