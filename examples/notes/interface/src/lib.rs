//! Interface layer of the `notes` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Notes` implementation and
//! will expose it over HTTP/gRPC once the framework runtime is wired (a later
//! phase). It never sees `domain` — the application service already mapped the
//! domain entity to the DTO (ADR 0003).

use notes_facade::{CreateNote, NoteDto, Notes};

/// Wraps a `Notes` implementation and exposes it. The concrete service comes
/// from `application`, chosen by the composition root in `apps/`.
pub struct NotesApi<S: Notes> {
    pub service: S,
}

impl<S: Notes> NotesApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /api/v1/notes` route will call.
    pub async fn create(&self, title: String) -> Result<NoteDto, String> {
        self.service.create(CreateNote { title }).await
    }
}
