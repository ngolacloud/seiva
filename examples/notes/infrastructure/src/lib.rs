//! Infrastructure layer of the `notes` module: adapters that implement the
//! module's outbound ports.
//!
//! Depends on `domain` (the repository port is the domain's — classic DDD, ADR
//! 0003) and std. In a real module this is the DORM-backed (or external)
//! repository; here it is an in-memory adapter so the wiring is provable without a
//! database.

use notes_domain::{Note, NoteRepository, RepositoryError};
use std::sync::Mutex;

/// An in-memory [`NoteRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryNoteRepository {
    saved: Mutex<Vec<Note>>,
}

impl InMemoryNoteRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every note saved so far — for composition-level assertions.
    pub fn saved(&self) -> Vec<Note> {
        self.saved.lock().expect("repository poisoned").clone()
    }
}

impl NoteRepository for InMemoryNoteRepository {
    async fn save(&self, note: &Note) -> Result<(), RepositoryError> {
        self.saved
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(note.clone());
        Ok(())
    }
}
