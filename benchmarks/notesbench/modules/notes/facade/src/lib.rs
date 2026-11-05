//! Facade layer: the module's versioned public contract and wire DTOs.
//!
//! This is what other modules and the HTTP/gRPC transports see. The
//! `#[seiva::facade]` macro (imported here as `facade`) keeps the trait as
//! the single source of truth and generates the `NotesFacade`
//! descriptor read by the OpenAPI/proto generators and the contract checks.
//!
//! The generated controller uses the facade's OWN DTOs as the wire types,
//! so the write DTO derives `serde` and implements `seiva_http::Validate`,
//! the response DTO derives `serde::Serialize`, and the error maps to an
//! RFC 9457 `Problem` here — the module owns its wire contract. Hence this
//! layer depends on `delonix-seiva`, `serde` and `seiva-http`.
#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};
use seiva_http::{Problem, Validate, ValidationError};
use serde::{Deserialize, Serialize};

/// Command DTO: the JSON body of `create`, validated by the controller's
/// `Valid<T>` extractor. Rename `Record`/`title` to the real shape.
/// `WireSchema` publishes its real fields to the OpenAPI document.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateRecord {
    /// The record's title; must be non-empty after trimming.
    pub title: String,
}

impl Validate for CreateRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}

/// Response DTO: what `create` returns. Never a domain type — the
/// application layer maps the domain entity to this at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, WireSchema)]
pub struct RecordDto {
    /// The record's identifier.
    pub id: u128,
    /// The record's title.
    pub title: String,
}

/// The facade's error type. The controller maps it to an RFC 9457 problem
/// via this `Into<Problem>`, so the module owns how failures surface.
#[derive(Debug, Clone)]
pub struct RecordError(pub String);

impl From<RecordError> for Problem {
    fn from(error: RecordError) -> Self {
        Problem::unprocessable_entity(error.0)
    }
}

/// The `notes` module's versioned public contract.
#[facade(version = "v1")]
pub trait Notes {
    /// Create a record (`POST /v1/notes`).
    ///
    /// BENCHMARK NOTE: the scaffold emits this guarded with
    /// `#[requires("notes.create")]`. The guard is removed HERE so the route
    /// is public, matching the no-auth FastAPI/Django apps this harness
    /// compares against — we measure framework overhead, not the auth layer.
    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError>;
}
