//! Application layer of the `notes` module: the `NotesService` (which
//! implements the public facade) and the `NoteRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item — the framework coupling is the facade's
//! concern (ADR 0003). The service consumes the framework's cross-cutting ports
//! (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through `seiva-ports`.

#![allow(async_fn_in_trait)]

use notes_domain::Note;
// Classic DDD (ADR 0003): the repository PORT is defined in the DOMAIN. Re-exported
// here so `interface` imports it from `application` (never naming `domain`), keeping
// the layer rule intact; the application maps its domain error to `PortResult`.
pub use notes_domain::{NoteRepository, RepositoryError};
use notes_facade::{CreateNote, NoteDto};
use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, Scope, UnitOfWork};

/// The application service. It runs the use case **under the caller's tenant
/// [`Scope`]** (SG1 / ADR 0029) and maps the domain entity to the DTO, exposing it
/// as an inherent `create` method. It does NOT implement the
/// [`Notes`](notes_facade::Notes) facade itself: that trait carries the HTTP `&Ctx`,
/// which this layer must not name (it is transport-free, ADR 0003). The interface
/// implements the facade, derives the `Scope` from the `&Ctx`, and delegates here.
pub struct NotesService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

impl<R, G, U, E> NotesService<R, G, U, E>
where
    R: NoteRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn create_note(&self, scope: Scope, cmd: CreateNote) -> PortResult<NoteDto> {
        // Validate in the domain first — no point opening a transaction for a
        // command the rule will reject.
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let note = Note::new(id, cmd.title).map_err(BoxError::from)?;

        // The event payload is a JSON envelope — the DORM gateway's outbox
        // publisher parses the bytes as JSON before persisting them, so a
        // non-JSON payload would fail against the real data layer.
        let event = serde_json::to_vec(&serde_json::json!({
            "id": note.id.to_string(),
            "title": note.title.clone(),
        }))
        .map_err(BoxError::from)?;

        // One transaction per use case (scope-based UnitOfWork, ADR 0004): the
        // save AND the event publish run inside it, so with the DORM gateway the
        // event lands in the transactional outbox in the SAME transaction as the
        // business change — they commit together or not at all. The relay worker
        // forwards the event to the broker afterwards. The `scope` is the caller's
        // tenant (SG1): the gateway binds it so the save lands under that tenant and
        // a forgotten scope fails closed at the gateway — never a fixed
        // `Scope::Global` (ADR 0029).
        self.uow
            .transaction(scope, || async {
                self.repository.save(&note).await?;
                self.events.publish("notes.created", &event).await
            })
            .await?;

        Ok(NoteDto {
            id: note.id,
            title: note.title,
        })
    }

    /// Creates a note under `scope` (the caller's tenant), mapping the internal
    /// error to the contract's `String` — no internal detail leaks across the
    /// module boundary beyond a message.
    ///
    /// This is an INHERENT method, not the [`Notes`](notes_facade::Notes) facade:
    /// the facade carries the HTTP `&Ctx`, which the application must not name (it
    /// stays transport-free). The interface implements the facade, derives the
    /// [`Scope`] from the `&Ctx` and calls this.
    pub async fn create(&self, scope: Scope, cmd: CreateNote) -> Result<NoteDto, String> {
        self.create_note(scope, cmd)
            .await
            .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seiva_testing::{
        InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator, TxOutcome,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        saved: Mutex<Vec<Note>>,
    }

    impl NoteRepository for FakeRepo {
        async fn save(&self, note: &Note) -> Result<(), RepositoryError> {
            self.saved.lock().unwrap().push(note.clone());
            Ok(())
        }
    }

    fn service(
    ) -> NotesService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        NotesService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    /// A tenant scope for the use case under test. The in-memory `UnitOfWork`
    /// ignores it (it has no tenant table), so these tests assert the use-case
    /// wiring; the DORM path proves the tenant binding end to end (`dorm-e2e`).
    fn scope() -> Scope {
        Scope::tenant("acme").expect("a non-empty tenant")
    }

    #[tokio::test]
    async fn create_persists_commits_and_emits() {
        let svc = service();
        let dto = svc
            .create(
                scope(),
                CreateNote {
                    title: "hello".into(),
                },
            )
            .await
            .unwrap();

        assert_eq!(dto.id, 1);
        assert_eq!(dto.title, "hello");
        assert_eq!(svc.repository.saved.lock().unwrap().len(), 1);
        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["notes.created"]);
    }

    #[tokio::test]
    async fn empty_title_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc
            .create(
                scope(),
                CreateNote {
                    title: "   ".into(),
                },
            )
            .await;
        assert!(err.is_err());
        // Nothing was committed or emitted when the rule rejected the command.
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }
}
