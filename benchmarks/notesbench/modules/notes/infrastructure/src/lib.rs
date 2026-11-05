//! Infrastructure layer: adapters implementing the application's outbound
//! ports. Depends on `application` + `domain` + `seiva-ports`; never on
//! `interface`.
//!
//! The default adapters are in-memory, so a scaffolded project compiles and
//! runs with NO database. A DORM-backed repository arrives via the `dorm`
//! feature (the `seiva-data-dorm` gateway) — swap this type for it in the
//! composition root when you turn the feature on. The adapters are `Clone`
//! (state behind an `Arc`) so the transport can hold the service as axum
//! state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use notes_application::RecordRepository;
use notes_domain::Record;
use seiva_ports::{Id, IdGenerator, PortResult};

/// An in-memory [`RecordRepository`]. Replace with the DORM-backed adapter
/// (feature `dorm`) or an external one for production.
#[derive(Clone, Default)]
pub struct InMemoryRecordRepository {
    saved: Arc<Mutex<Vec<Record>>>,
}

impl InMemoryRecordRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every record saved so far — for composition-level assertions.
    pub fn saved(&self) -> Vec<Record> {
        self.saved.lock().expect("repository poisoned").clone()
    }
}

impl RecordRepository for InMemoryRecordRepository {
    async fn save(&self, record: &Record) -> PortResult<()> {
        self.saved
            .lock()
            .map_err(|_| "repository lock poisoned")?
            .push(record.clone());
        Ok(())
    }
}

/// A process-local monotonic [`IdGenerator`] adapter: it hands out
/// 1, 2, 3, … as 128-bit ids. Replace with a ULID/UUID generator in
/// production; this keeps the scaffold dependency-free and deterministic.
#[derive(Clone, Default)]
pub struct CounterIdGenerator {
    next: Arc<AtomicU64>,
}

impl IdGenerator for CounterIdGenerator {
    fn new_id(&self) -> Id {
        let value = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut bytes = [0u8; 16];
        bytes[8..].copy_from_slice(&value.to_be_bytes());
        Id(bytes)
    }
}
