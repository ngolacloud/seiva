//! Infrastructure layer of the `notes` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In a real module this
//! is the DORM-backed (or external) repository; here it is an in-memory adapter
//! so the wiring is provable without a database.

use notes_application::NoteRepository;
use notes_domain::Note;
use seiva_ports::PortResult;
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
    async fn save(&self, note: &Note) -> PortResult<()> {
        self.saved
            .lock()
            .map_err(|_| "repository lock poisoned")?
            .push(note.clone());
        Ok(())
    }
}
