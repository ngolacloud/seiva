//! Facade layer of the `notes` module: the versioned public contract and DTOs.
//!
//! This is what other modules see. The `#[seiva::facade]` macro keeps the trait
//! as the source of truth and generates the `NotesFacade` descriptor (ADR 0001).

#![allow(async_fn_in_trait)]

use delonix_seiva::facade;

/// Command DTO: the input to `create`.
#[derive(Debug, Clone)]
pub struct CreateNote {
    pub title: String,
}

/// Response DTO: what `create`/`get` return. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDto {
    pub id: u128,
    pub title: String,
}

/// The `notes` module's public contract.
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}
