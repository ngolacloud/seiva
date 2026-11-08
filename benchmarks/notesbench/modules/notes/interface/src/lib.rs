//! Interface layer: the HTTP/gRPC transport. It wires this module's facade
//! to the real framework transports.
//!
//! LAYERING DECISION: the `#[seiva::controller]` lives HERE, not in
//! `application`, so the application layer stays transport-free. The
//! controller [`NotesController<R, G>`] is GENERIC over the repository
//! and id-generator ports, so this layer depends only on `application` (the
//! service and the `RecordRepository` port) and NEVER on `infrastructure` —
//! the layering rule forbids `interface -> infrastructure`. A concrete self
//! type is still what `#[seiva::controller]` dispatches through, but
//! `NotesController<R, G>` is concrete once the ports are chosen, and the
//! macro propagates the generics + their bounds onto the generated router.
//!
//! The macro generates a `notes_router(service)` builder whose route
//! version and per-route permission come from the facade descriptor at run
//! time. The COMPOSITION ROOT in `apps/` picks the concrete adapters and
//! calls `notes_router(NotesController::new(repo, ids))`. The same
//! descriptor drives this module's OpenAPI document ([`openapi`]), so the
//! wire contract is generated, never hand-maintained.
//!
//! The generated handlers return `Result<_, seiva_http::Problem>`, a large
//! value, so the large-`Err` lint is allowed crate-wide (as `seiva-http`
//! itself does).
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use delonix_seiva::contract::Facade;
use delonix_seiva::controller;
use seiva_ports::IdGenerator;
use notes_application::{RecordRepository, RecordService};
use notes_facade::{CreateRecord, RecordDto, RecordError, Notes, NotesFacade};

/// Transport adapter: a concrete `Notes` service exposed over HTTP,
/// generic over the repository port `R` and the id-generator port `G`.
/// Held behind an [`Arc`] so it is `Clone + Send + Sync + 'static` — the
/// bound the generated `notes_router` places on its axum state — without
/// requiring the service or its ports to be `Clone`.
pub struct NotesController<R, G>(Arc<RecordService<R, G>>);

// Manual `Clone` (not derived): the controller is `Clone` for ANY `R`/`G`
// because it only holds an `Arc`; the derive would wrongly demand both be
// `Clone`.
impl<R, G> Clone for NotesController<R, G> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: RecordRepository, G: IdGenerator> NotesController<R, G> {
    /// Builds the controller over the given ports. The composition root in
    /// `apps/` chooses them (the in-memory adapters, or DORM-backed ones); the
    /// interface never names a concrete adapter.
    pub fn new(repository: R, ids: G) -> Self {
        NotesController(Arc::new(RecordService { repository, ids }))
    }
}

#[controller]
impl<R, G> Notes for NotesController<R, G>
where
    R: RecordRepository + Send + Sync + 'static,
    G: IdGenerator + Send + Sync + 'static,
{
    async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, RecordError> {
        self.0.create(cmd).await
    }
}

/// The module's OpenAPI 3.1 document, generated from the facade descriptor.
/// (The framework can also derive a proto3 document from the same descriptor;
/// this benchmark only exercises the OpenAPI path, keeping the crate free of
/// the gRPC generator.)
pub fn openapi() -> serde_json::Value {
    seiva_openapi::openapi_document(
        seiva_openapi::ApiInfo {
            title: "notes API".to_string(),
            version: NotesFacade::DESCRIPTOR.version.to_string(),
        },
        &[NotesFacade::DESCRIPTOR],
    )
}
