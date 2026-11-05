//! Facade layer of the `polls` module: the versioned public contract and DTOs.
//!
//! This is the Seiva port of the application built in the Django "Writing your
//! first Django app" tutorial — a polls service with questions and choices you
//! can vote on. In Django the tutorial is driven by models, views, templates
//! and the admin; in Seiva the module exposes **one machine-readable contract**
//! (`#[seiva::facade]`) and the HTTP routes, the OpenAPI document and the gRPC
//! service are all derived from it.
//!
//! The Django analogues:
//!
//! - `Question` / `Choice` models  → the [`QuestionDto`] / [`ChoiceDto`] wire DTOs
//!   (the entities themselves live in `polls-domain`).
//! - the index / detail / results views → [`Polls::list`] / [`Polls::detail`]
//!   / [`Polls::results`].
//! - the `vote()` view                  → [`Polls::vote`].

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// Command DTO: create a question (Django: `Question(question_text=…)`).
#[derive(Debug, Clone, WireSchema)]
pub struct CreateQuestion {
    pub text: String,
}

/// Command DTO: add a choice to a question (Django: a `Choice` with a FK to the
/// `Question`).
#[derive(Debug, Clone, WireSchema)]
pub struct AddChoice {
    pub question_id: u128,
    pub text: String,
}

/// Command DTO: cast a vote for a choice (Django: the `vote()` view that bumps
/// `choice.votes`).
#[derive(Debug, Clone, WireSchema)]
pub struct Vote {
    pub choice_id: u128,
}

/// Query DTO: identify a question (Django: the `<question_id>` URL capture).
#[derive(Debug, Clone, WireSchema)]
pub struct GetQuestion {
    pub id: u128,
}

/// Response DTO for a question. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct QuestionDto {
    pub id: u128,
    pub text: String,
}

/// Response DTO for a choice, carrying its running vote tally.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct ChoiceDto {
    pub id: u128,
    pub question_id: u128,
    pub text: String,
    pub votes: u64,
}

/// Response DTO for a question together with its choices — the shape the Django
/// detail and results pages render.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct QuestionDetail {
    pub question: QuestionDto,
    pub choices: Vec<ChoiceDto>,
}

/// The `polls` module's public contract. Each method declares the permission a
/// caller must hold; the transport enforces it (deny by default, ADR 0005).
#[facade(version = "v1")]
pub trait Polls {
    /// Create a question. (Django: add a `Question` in the admin or shell.)
    #[requires("polls.manage")]
    async fn ask(&self, cmd: CreateQuestion) -> Result<QuestionDto, String>;

    /// Add a choice to an existing question.
    #[requires("polls.manage")]
    async fn add_choice(&self, cmd: AddChoice) -> Result<ChoiceDto, String>;

    /// The latest questions. (Django tutorial Part 3: the index view.)
    #[requires("polls.read")]
    async fn list(&self) -> Result<Vec<QuestionDto>, String>;

    /// A question and its choices. (Django: the detail view.)
    #[requires("polls.read")]
    async fn detail(&self, q: GetQuestion) -> Result<QuestionDetail, String>;

    /// Cast a vote for a choice and return the updated tally. (Django: `vote()`.)
    #[requires("polls.vote")]
    async fn vote(&self, cmd: Vote) -> Result<ChoiceDto, String>;

    /// A question with the current vote tallies. (Django: the results view.)
    #[requires("polls.read")]
    async fn results(&self, q: GetQuestion) -> Result<QuestionDetail, String>;
}
