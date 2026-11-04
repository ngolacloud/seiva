//! Standalone proof that `#[seiva::controller]` generates a facade-driven axum
//! router.
//!
//! It defines a small `Widgets` facade with `#[seiva::facade]`, a fake service
//! implementing it, and annotates that impl with `#[seiva::controller]` so the
//! macro emits a `widgets_router` builder. The integration tests in `tests/`
//! drive that router with `tower::ServiceExt::oneshot` and assert the
//! descriptor-driven behaviour (version in the path, per-route permission guard,
//! RFC 9457 problems).
//!
//! Both macros are used through their public path on `delonix-seiva`
//! (`#[seiva::facade]`, `#[seiva::controller]`), the way a downstream module
//! sees them — the proc-macro crate is an implementation detail.

#![forbid(unsafe_code)]
// The generated handlers return `Result<_, seiva_http::Problem>`, and `Problem`
// is a rich (large) RFC 9457 payload returned by value on the error path — that
// is the transport's design, so the large-Err lint is allowed crate-wide, as
// `seiva-http` itself does.
#![allow(clippy::result_large_err)]

use serde::{Deserialize, Serialize};

use delonix_seiva::{controller, facade};
use seiva_http::{Problem, Validate, ValidationError};

/// Write DTO for `create`. Deserialised from the JSON body and validated by the
/// `Valid<T>` extractor the controller generates.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateWidget {
    /// The widget's name; must be non-empty after trimming.
    pub name: String,
}

impl Validate for CreateWidget {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::new("name must not be empty"));
        }
        Ok(())
    }
}

/// Response DTO returned by `create`/`get`/`list`. Serialised to JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WidgetDto {
    /// The widget's identifier.
    pub id: u64,
    /// The widget's name.
    pub name: String,
}

/// The facade's error type. The controller maps it to an RFC 9457 problem via
/// the `Into<Problem>` bound it requires, so the interface owns the mapping.
#[derive(Debug, Clone)]
pub struct WidgetError(pub String);

impl From<WidgetError> for Problem {
    fn from(error: WidgetError) -> Self {
        Problem::unprocessable_entity(error.0)
    }
}

/// The `widgets` module's public contract. Version `v2` proves the generated
/// route path is read from the descriptor, not a hard-coded `v1`.
#[facade(version = "v2")]
pub trait Widgets {
    /// Create a widget (guarded, `POST /v2/widgets`).
    #[requires("widgets.create")]
    async fn create(&self, cmd: CreateWidget) -> Result<WidgetDto, WidgetError>;

    /// Fetch one widget by id (guarded, `GET /v2/widgets/{id}`).
    #[requires("widgets.read")]
    async fn get(&self, id: u64) -> Result<WidgetDto, WidgetError>;

    /// List widgets (unguarded, `GET /v2/widgets`).
    async fn list(&self) -> Result<Vec<WidgetDto>, WidgetError>;
}

/// A fake, in-memory service standing in for a real application use case. It is
/// `Clone` so the generated router can hold it as axum state.
#[derive(Debug, Clone, Default)]
pub struct WidgetService;

#[controller]
impl Widgets for WidgetService {
    async fn create(&self, cmd: CreateWidget) -> Result<WidgetDto, WidgetError> {
        Ok(WidgetDto {
            id: 1,
            name: cmd.name,
        })
    }

    async fn get(&self, id: u64) -> Result<WidgetDto, WidgetError> {
        Ok(WidgetDto {
            id,
            name: "fetched".to_owned(),
        })
    }

    async fn list(&self) -> Result<Vec<WidgetDto>, WidgetError> {
        Ok(vec![WidgetDto {
            id: 1,
            name: "one".to_owned(),
        }])
    }
}
