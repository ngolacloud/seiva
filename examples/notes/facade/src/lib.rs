//! Facade layer of the `notes` module: the versioned public contract and DTOs.
//!
//! This is what other modules see. The `#[seiva::facade]` macro keeps the trait
//! as the source of truth and generates the `NotesFacade` descriptor (ADR 0001).

#![allow(async_fn_in_trait)]

use seiva::{facade, WireSchema};
use seiva_http::Ctx;

/// Command DTO: the input to `create`.
#[derive(Debug, Clone, WireSchema)]
pub struct CreateNote {
    pub title: String,
}

/// Response DTO: what `create`/`get` return. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct NoteDto {
    pub id: u128,
    pub title: String,
}

/// The `notes` module's public contract.
///
/// `create` takes the framework request context `&Ctx` (the authenticated caller:
/// tenant + permissions). The `#[seiva::controller]` macro **skips it from the
/// wire** (ADR 0006) and injects it from the request, so it never appears in a DTO;
/// the interface derives the tenant [`Scope`](seiva_http) from it **server-side** —
/// the tenant is never read from the request body (OWASP API1), and the use case
/// runs under that tenant, never a fixed `Scope::Global` (SG1 / ADR 0029). The
/// application stays HTTP-free: it receives a `Scope`, not a `Ctx`.
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto, String>;
}
