//! End-to-end proof (ADR 0030 done-criterion): a real `examples/notes` module use
//! case runs through the SeaORM adapter — a write inside `UnitOfWork::transaction`,
//! then a read-back — with the module's repository finding the ambient transaction
//! via `with_current_tx`. The application and domain layers name no ORM; only this
//! test's infrastructure adapter (a `NoteRepository`) touches SeaORM.
//!
//! Gated on `SEIVA_SEAORM_TEST_DSN`; skips cleanly when unset.

#![cfg(feature = "seaorm")]

use notes_application::{NotesService, RepositoryError};
use notes_domain::{Note, NoteRepository};
use notes_facade::CreateNote;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};
use seiva_data_seaorm::{
    connect, with_current_tx, SeaOrmConfig, SeaOrmOutboxPublisher, SeaOrmUnitOfWork,
};
use seiva_ports::{BoxError, Scope};
use seiva_testing::SequenceIdGenerator;
use std::future::Future;

/// A SeaORM-backed `NoteRepository` — the module's `infrastructure` adapter. It
/// holds no handle: it reaches the unit of work's live transaction through the
/// adapter's ambient-transaction seam, so the save joins the same transaction as
/// the event.
struct SeaOrmNoteRepository;

impl NoteRepository for SeaOrmNoteRepository {
    fn save(&self, note: &Note) -> impl Future<Output = Result<(), RepositoryError>> + Send {
        // u128 ids do not fit a bigint, so the schema stores the id as text.
        let id = note.id.to_string();
        let title = note.title.clone();
        async move {
            with_current_tx(move |txn| {
                Box::pin(async move {
                    txn.execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "INSERT INTO notes (id, title) VALUES ($1, $2)",
                        vec![Value::String(Some(id)), Value::String(Some(title))],
                    ))
                    .await
                    .map(|_| ())
                    .map_err(|e| Box::new(e) as BoxError)
                })
            })
            .await
            .map_err(|e| RepositoryError(e.to_string()))
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_create_persists_and_reads_back_through_the_seaorm_adapter() {
    let Ok(dsn) = std::env::var("SEIVA_SEAORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_SEAORM_TEST_DSN to run the notes-over-SeaORM proof");
        return;
    };
    let conn = connect(&SeaOrmConfig { dsn, pool_max: 8 })
        .await
        .expect("connect");

    // The module's schema (owned by the user) + the framework outbox.
    conn.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS notes (id text primary key, title text not null)",
    )
    .await
    .expect("notes table");
    for stmt in seiva_data_seaorm::outbox_migrations() {
        conn.execute_unprepared(&stmt).await.expect("outbox DDL");
    }
    conn.execute_unprepared("DELETE FROM notes")
        .await
        .expect("clean");
    conn.execute_unprepared("DELETE FROM seiva_outbox WHERE topic = 'notes.created'")
        .await
        .expect("clean outbox");

    // The real NotesService over the SeaORM ports.
    let service = NotesService {
        repository: SeaOrmNoteRepository,
        ids: SequenceIdGenerator::new(),
        uow: SeaOrmUnitOfWork::new(conn.clone()),
        events: SeaOrmOutboxPublisher,
    };

    // Runs under the caller's tenant `Scope` (SG1 / ADR 0029), derived server-side
    // by the interface; the SeaORM unit of work carries it through, so the use case
    // no longer hard-codes `Scope::Global`.
    let dto = service
        .create(
            Scope::tenant("acme").expect("a non-empty tenant"),
            CreateNote {
                title: "hello from seaorm".into(),
            },
        )
        .await
        .expect("create_note succeeds");
    assert_eq!(dto.id, 1);

    // The write committed: read the row back.
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT title FROM notes WHERE id = $1",
            vec![Value::String(Some(dto.id.to_string()))],
        ))
        .await
        .expect("query")
        .expect("the note row is present after commit");
    assert_eq!(
        row.try_get_by_index::<String>(0).unwrap(),
        "hello from seaorm"
    );

    // The event landed in the same transaction (transactional outbox).
    let events = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*)::int8 FROM seiva_outbox WHERE topic = 'notes.created'",
        ))
        .await
        .expect("query")
        .expect("one row")
        .try_get_by_index::<i64>(0)
        .unwrap();
    assert_eq!(events, 1, "the created event committed with the write");

    conn.execute_unprepared("DELETE FROM notes").await.ok();
    conn.execute_unprepared("DELETE FROM seiva_outbox WHERE topic = 'notes.created'")
        .await
        .ok();
}
