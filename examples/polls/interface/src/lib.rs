//! Interface layer of the `polls` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Polls` implementation and
//! exposes it. It never sees `domain` — the application service already mapped
//! the domain entities to DTOs (ADR 0003). In a scaffolded module the
//! `#[seiva::controller]` macro generates the axum routes (`GET /v1/polls`,
//! `POST /v1/polls/vote`, …) from the facade; this example keeps a plain
//! wrapper so the layering is provable without the transport runtime.

use polls_facade::{
    AddChoice, ChoiceDto, CreateQuestion, GetQuestion, Polls, QuestionDetail, QuestionDto, Vote,
};

/// Wraps a `Polls` implementation and exposes it. The concrete service comes
/// from `application`, chosen by the composition root in `apps/`.
pub struct PollsApi<S: Polls> {
    pub service: S,
}

impl<S: Polls> PollsApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/polls/questions` route will call.
    pub async fn ask(&self, text: String) -> Result<QuestionDto, String> {
        self.service.ask(CreateQuestion { text }).await
    }

    /// The handler a `POST /v1/polls/choices` route will call.
    pub async fn add_choice(&self, question_id: u128, text: String) -> Result<ChoiceDto, String> {
        self.service
            .add_choice(AddChoice { question_id, text })
            .await
    }

    /// The handler the `GET /v1/polls` index route will call.
    pub async fn list(&self) -> Result<Vec<QuestionDto>, String> {
        self.service.list().await
    }

    /// The handler the `GET /v1/polls/{id}` detail route will call.
    pub async fn detail(&self, id: u128) -> Result<QuestionDetail, String> {
        self.service.detail(GetQuestion { id }).await
    }

    /// The handler the `POST /v1/polls/vote` route will call.
    pub async fn vote(&self, choice_id: u128) -> Result<ChoiceDto, String> {
        self.service.vote(Vote { choice_id }).await
    }

    /// The handler the `GET /v1/polls/{id}/results` route will call.
    pub async fn results(&self, id: u128) -> Result<QuestionDetail, String> {
        self.service.results(GetQuestion { id }).await
    }
}
