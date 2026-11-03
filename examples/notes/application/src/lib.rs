//! Application layer of the `notes` module: the `NotesService` (which
//! implements the public facade) and the `NoteRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item — the framework coupling is the facade's
//! concern (ADR 0003). The service consumes the framework's cross-cutting ports
//! (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through `seiva-ports`.

#![allow(async_fn_in_trait)]

use notes_domain::Note;
use notes_facade::{CreateNote, NoteDto, Notes};
use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, UnitOfWork};

/// Outbound port: persistence of notes, in the domain's language. Implemented
/// in `infrastructure` (over DORM or an external library). The future is `Send`
/// so a use case can `.await` it inside a `UnitOfWork::transaction`, which runs
/// on a multi-threaded runtime.
pub trait NoteRepository {
    fn save(&self, note: &Note) -> impl std::future::Future<Output = PortResult<()>> + Send;
}

/// The application service. It implements the [`Notes`] facade by running the
/// use case and mapping the domain entity to the DTO.
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
    async fn create_note(&self, cmd: CreateNote) -> PortResult<NoteDto> {
        // Validate in the domain first — no point opening a transaction for a
        // command the rule will reject.
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let note = Note::new(id, cmd.title).map_err(BoxError::from)?;

        // One transaction per use case: the save runs inside it and the port
        // commits iff the body returns Ok (scope-based UnitOfWork, ADR 0004).
        self.uow
            .transaction(|| async { self.repository.save(&note).await })
            .await?;

        // Publish after a successful commit (the relay model); the id bytes
        // stand in for a serialized event envelope.
        self.events
            .publish("notes.created", &note.id.to_be_bytes())
            .await?;

        Ok(NoteDto {
            id: note.id,
            title: note.title,
        })
    }
}

impl<R, G, U, E> Notes for NotesService<R, G, U, E>
where
    R: NoteRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String> {
        // Map the internal error to the contract's error type: no internal
        // detail leaks across the module boundary beyond a message.
        self.create_note(cmd).await.map_err(|err| err.to_string())
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
        async fn save(&self, note: &Note) -> PortResult<()> {
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

    #[tokio::test]
    async fn create_persists_commits_and_emits() {
        let svc = service();
        let dto = svc
            .create(CreateNote {
                title: "hello".into(),
            })
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
            .create(CreateNote {
                title: "   ".into(),
            })
            .await;
        assert!(err.is_err());
        // Nothing was committed or emitted when the rule rejected the command.
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }
}
