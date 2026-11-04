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

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

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

/// The JSON body of a health response.
#[derive(Debug, Serialize)]
struct HealthBody {
    status: &'static str,
    state: &'static str,
}

/// `GET /health/live` — alive in every state, so a draining process is not
/// killed mid-shutdown.
async fn live(State(health): State<Health>) -> (StatusCode, Json<HealthBody>) {
    (
        StatusCode::OK,
        Json(HealthBody {
            status: "live",
            state: health.state().as_str(),
        }),
    )
}

/// `GET /health/ready` — `200` only while [`HealthState::Ready`], else `503`.
async fn ready(State(health): State<Health>) -> (StatusCode, Json<HealthBody>) {
    let state = health.state();
    let status = if state == HealthState::Ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(HealthBody {
            status: if state == HealthState::Ready {
                "ready"
            } else {
                "unready"
            },
            state: state.as_str(),
        }),
    )
}

/// Builds the health sub-router, backed by the given [`Health`] handle.
pub fn health_router(health: Health) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .with_state(health)
}
