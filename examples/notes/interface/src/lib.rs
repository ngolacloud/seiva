//! Interface layer of the `notes` module: the transport-facing shell.
//!
//! It wraps the application's [`NotesService`] and implements the `Notes` facade,
//! deriving the caller's tenant [`Scope`] from the request `&Ctx` **server-side**
//! (SG1 / ADR 0029) before delegating to the service's inherent, HTTP-free
//! `create`. It depends on `application` (for the service and the re-exported
//! `NoteRepository` port) and the HTTP kernel — never on `domain` (the layer rule
//! forbids `interface -> domain`; the port is imported from `application`, classic
//! DDD re-export) and never on `infrastructure`.
//!
//! The concrete repository is chosen by the composition root in `apps/`, which
//! builds a [`NotesService`] and wraps it in [`NotesApi::new`].

#![allow(async_fn_in_trait)]

// The repository port is the domain's (classic DDD, ADR 0003), re-exported by the
// application so the interface imports it from there and never names `domain`.
use notes_application::{NoteRepository, NotesService};
use notes_facade::{CreateNote, NoteDto, Notes};
use seiva_http::Ctx;
use seiva_ports::{EventPublisher, IdGenerator, Scope, UnitOfWork};

/// Derives the caller's tenant [`Scope`] from the request context, **fail-closed**
/// (SG1): an untenanted or anonymous caller gets an error, never a global/unscoped
/// write. The tenant comes from the authenticated `Principal` (server-side), never
/// from the request body (OWASP API1). Passed to the application so every statement
/// the use case runs is bound to this tenant — the honour-system `Scope::Global`
/// the ADR 0029 lint now forbids is gone.
fn scope_of(ctx: &Ctx) -> Result<Scope, String> {
    ctx.tenant()
        .and_then(Scope::tenant)
        .ok_or_else(|| "caller has no tenant".to_owned())
}

/// Wraps a [`NotesService`] and exposes it as the `Notes` facade. The concrete
/// service (its repository + in-process ports) comes from the composition root in
/// `apps/`.
pub struct NotesApi<R, G, U, E> {
    pub service: NotesService<R, G, U, E>,
}

impl<R, G, U, E> NotesApi<R, G, U, E> {
    pub fn new(service: NotesService<R, G, U, E>) -> Self {
        Self { service }
    }
}

impl<R, G, U, E> Notes for NotesApi<R, G, U, E>
where
    R: NoteRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    /// The `&Ctx` is injected by the transport (skipped from the wire, ADR 0006);
    /// we derive the tenant [`Scope`] from it here and hand the application a
    /// `Scope`, keeping it HTTP-free.
    async fn create(&self, ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto, String> {
        self.service.create(scope_of(ctx)?, cmd).await
    }
}
