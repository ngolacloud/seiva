//! End-to-end proof (ADR 0030 done-criterion): a real `examples/notes` module use
//! case runs through the sqlx adapter — a write inside `UnitOfWork::transaction`,
//! then a read-back — with the module's repository finding the ambient transaction
//! via `with_current_tx`. The application and domain layers name no ORM; only this
//! test's infrastructure adapter (a `NoteRepository`) touches sqlx.
//!
//! Gated on `SEIVA_SQLX_TEST_DSN`; skips cleanly when unset.

#![cfg(feature = "sqlx")]

use notes_application::{NotesService, RepositoryError};
use notes_domain::{Note, NoteRepository};
use notes_facade::{CreateNote, Notes};
use seiva_data_sqlx::{connect, with_current_tx, SqlxConfig, SqlxOutboxPublisher, SqlxUnitOfWork};
use seiva_ports::BoxError;
use seiva_testing::SequenceIdGenerator;
use sqlx::Row;
use std::future::Future;

/// A sqlx-backed `NoteRepository` — the module's `infrastructure` adapter. It holds
/// no handle: it reaches the unit of work's live transaction through the adapter's
/// ambient-transaction seam, so the save joins the same transaction as the event.
struct SqlxNoteRepository;

impl NoteRepository for SqlxNoteRepository {
    fn save(&self, note: &Note) -> impl Future<Output = Result<(), RepositoryError>> + Send {
        // u128 ids do not fit a bigint, so the schema stores the id as text.
        let id = note.id.to_string();
        let title = note.title.clone();
        async move {
            with_current_tx(move |tx| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO notes (id, title) VALUES ($1, $2)")
                        .bind(id)
                        .bind(title)
                        .execute(&mut **tx)
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
async fn notes_create_persists_and_reads_back_through_the_sqlx_adapter() {
    let Ok(dsn) = std::env::var("SEIVA_SQLX_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_SQLX_TEST_DSN to run the notes-over-sqlx proof");
        return;
    };
    let pool = connect(&SqlxConfig { dsn, pool_max: 8 })
        .await
        .expect("connect");

    // The module's schema (owned by the user; sqlx brings no migrations) + the
    // framework outbox the event publisher writes to.
    sqlx::query("CREATE TABLE IF NOT EXISTS notes (id text primary key, title text not null)")
        .execute(&pool)
        .await
        .expect("notes table");
    for stmt in seiva_data_sqlx::outbox_migrations() {
        sqlx::query(sqlx::AssertSqlSafe(stmt))
            .execute(&pool)
            .await
            .expect("outbox DDL");
    }
    sqlx::query("DELETE FROM notes")
        .execute(&pool)
        .await
        .expect("clean notes");
    sqlx::query("DELETE FROM seiva_outbox WHERE topic = 'notes.created'")
        .execute(&pool)
        .await
        .expect("clean outbox");

    // Wire the real NotesService against the sqlx ports. Only the repository and
    // the unit of work / publisher are sqlx-backed; the service is the shipped one.
    let service = NotesService {
        repository: SqlxNoteRepository,
        ids: SequenceIdGenerator::new(),
        uow: SqlxUnitOfWork::new(pool.clone()),
        events: SqlxOutboxPublisher,
    };

    // The use case opens one transaction, saves the note AND writes the event into
    // the outbox inside it — commit together.
    let dto = service
        .create(CreateNote {
            title: "hello from sqlx".into(),
        })
        .await
        .expect("create_note succeeds");
    assert_eq!(dto.id, 1, "SequenceIdGenerator hands out id 1 first");

    // Read the row back: the write committed.
    let title: String = sqlx::query("SELECT title FROM notes WHERE id = $1")
        .bind(dto.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("the note row is present after commit")
        .get("title");
    assert_eq!(title, "hello from sqlx");

    // The event landed in the same transaction (transactional outbox).
    let events: i64 =
        sqlx::query("SELECT count(*) AS n FROM seiva_outbox WHERE topic = 'notes.created'")
            .fetch_one(&pool)
            .await
            .expect("count")
            .get("n");
    assert_eq!(events, 1, "the created event committed with the write");

    // Cleanup.
    sqlx::query("DELETE FROM notes").execute(&pool).await.ok();
    sqlx::query("DELETE FROM seiva_outbox WHERE topic = 'notes.created'")
        .execute(&pool)
        .await
        .ok();
}
