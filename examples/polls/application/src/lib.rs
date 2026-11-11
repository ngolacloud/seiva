//! Application layer of the `polls` module: the `PollsService` (which implements
//! the public facade) and the `PollRepository` outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item — the framework coupling is the facade's
//! concern (ADR 0003). The service consumes the framework's cross-cutting ports
//! (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through `seiva-ports`.
//!
//! This is the Seiva equivalent of the Django tutorial's views: `list` is the
//! index view, `detail`/`results` are the detail/results views, and `vote`
//! bumps a choice's tally inside a single transaction.

#![allow(async_fn_in_trait)]

use polls_domain::{Choice, Question};
pub use polls_domain::{PollRepository, RepositoryError};
use polls_facade::{
    AddChoice, ChoiceDto, CreateQuestion, GetQuestion, Polls, QuestionDetail, QuestionDto, Vote,
};
use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, Scope, UnitOfWork};

/// The application service. It implements the [`Polls`] facade by running each
/// use case over the ports and mapping the domain entities to DTOs.
pub struct PollsService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_question_dto(q: Question) -> QuestionDto {
    QuestionDto {
        id: q.id,
        text: q.text,
    }
}

fn to_choice_dto(c: Choice) -> ChoiceDto {
    ChoiceDto {
        id: c.id,
        question_id: c.question_id,
        text: c.text,
        votes: c.votes,
    }
}

impl<R, G, U, E> PollsService<R, G, U, E>
where
    R: PollRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn ask_question(&self, cmd: CreateQuestion) -> PortResult<QuestionDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let question = Question::new(id, cmd.text).map_err(BoxError::from)?;

        let event = serde_json::to_vec(&serde_json::json!({
            "id": question.id.to_string(),
            "text": question.text.clone(),
        }))
        .map_err(BoxError::from)?;

        // One transaction per use case (ADR 0004): the save AND the event
        // publish run inside it, so with the DORM gateway the event lands in the
        // transactional outbox in the SAME transaction as the business change.
        self.uow
            .transaction(Scope::Global, || async {
                self.repository.save_question(&question).await?;
                self.events.publish("polls.question_asked", &event).await
            })
            .await?;

        Ok(to_question_dto(question))
    }

    async fn add_a_choice(&self, cmd: AddChoice) -> PortResult<ChoiceDto> {
        // A choice cannot dangle: the question must exist first.
        if self
            .repository
            .get_question(cmd.question_id)
            .await?
            .is_none()
        {
            return Err(BoxError::from(format!(
                "question {} not found",
                cmd.question_id
            )));
        }

        let id = u128::from_be_bytes(self.ids.new_id().0);
        let choice = Choice::new(id, cmd.question_id, cmd.text).map_err(BoxError::from)?;

        let event = serde_json::to_vec(&serde_json::json!({
            "id": choice.id.to_string(),
            "question_id": choice.question_id.to_string(),
            "text": choice.text.clone(),
        }))
        .map_err(BoxError::from)?;

        self.uow
            .transaction(Scope::Global, || async {
                self.repository.save_choice(&choice).await?;
                self.events.publish("polls.choice_added", &event).await
            })
            .await?;

        Ok(to_choice_dto(choice))
    }

    async fn latest_questions(&self) -> PortResult<Vec<QuestionDto>> {
        let questions = self.repository.list_questions().await?;
        Ok(questions.into_iter().map(to_question_dto).collect())
    }

    async fn question_detail(&self, id: u128) -> PortResult<QuestionDetail> {
        let question = self
            .repository
            .get_question(id)
            .await?
            .ok_or_else(|| BoxError::from(format!("question {id} not found")))?;
        let choices = self.repository.choices_for(id).await?;
        Ok(QuestionDetail {
            question: to_question_dto(question),
            choices: choices.into_iter().map(to_choice_dto).collect(),
        })
    }

    async fn cast_a_vote(&self, cmd: Vote) -> PortResult<ChoiceDto> {
        let choice = self
            .uow
            .transaction(Scope::Global, || async {
                let choice = self
                    .repository
                    .record_vote(cmd.choice_id)
                    .await?
                    .ok_or_else(|| BoxError::from(format!("choice {} not found", cmd.choice_id)))?;

                let event = serde_json::to_vec(&serde_json::json!({
                    "choice_id": choice.id.to_string(),
                    "question_id": choice.question_id.to_string(),
                    "votes": choice.votes,
                }))
                .map_err(BoxError::from)?;
                self.events.publish("polls.voted", &event).await?;

                Ok(choice)
            })
            .await?;

        Ok(to_choice_dto(choice))
    }
}

impl<R, G, U, E> Polls for PollsService<R, G, U, E>
where
    R: PollRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    // Each method maps the internal error to the contract's error type, so no
    // internal detail leaks across the module boundary beyond a message.
    async fn ask(&self, cmd: CreateQuestion) -> Result<QuestionDto, String> {
        self.ask_question(cmd).await.map_err(|err| err.to_string())
    }

    async fn add_choice(&self, cmd: AddChoice) -> Result<ChoiceDto, String> {
        self.add_a_choice(cmd).await.map_err(|err| err.to_string())
    }

    async fn list(&self) -> Result<Vec<QuestionDto>, String> {
        self.latest_questions().await.map_err(|err| err.to_string())
    }

    async fn detail(&self, q: GetQuestion) -> Result<QuestionDetail, String> {
        self.question_detail(q.id)
            .await
            .map_err(|err| err.to_string())
    }

    async fn vote(&self, cmd: Vote) -> Result<ChoiceDto, String> {
        self.cast_a_vote(cmd).await.map_err(|err| err.to_string())
    }

    async fn results(&self, q: GetQuestion) -> Result<QuestionDetail, String> {
        // Results and detail share a shape; the permission is the difference a
        // caller sees, and both are `polls.read` here.
        self.question_detail(q.id)
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
        questions: Mutex<Vec<Question>>,
        choices: Mutex<Vec<Choice>>,
    }

    impl PollRepository for FakeRepo {
        async fn save_question(&self, question: &Question) -> Result<(), RepositoryError> {
            self.questions.lock().unwrap().push(question.clone());
            Ok(())
        }

        async fn save_choice(&self, choice: &Choice) -> Result<(), RepositoryError> {
            self.choices.lock().unwrap().push(choice.clone());
            Ok(())
        }

        async fn list_questions(&self) -> Result<Vec<Question>, RepositoryError> {
            Ok(self.questions.lock().unwrap().clone())
        }

        async fn get_question(&self, id: u128) -> Result<Option<Question>, RepositoryError> {
            Ok(self
                .questions
                .lock()
                .unwrap()
                .iter()
                .find(|q| q.id == id)
                .cloned())
        }

        async fn choices_for(&self, question_id: u128) -> Result<Vec<Choice>, RepositoryError> {
            Ok(self
                .choices
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.question_id == question_id)
                .cloned()
                .collect())
        }

        async fn record_vote(&self, choice_id: u128) -> Result<Option<Choice>, RepositoryError> {
            let mut choices = self.choices.lock().unwrap();
            match choices.iter_mut().find(|c| c.id == choice_id) {
                Some(choice) => {
                    choice.cast_vote();
                    Ok(Some(choice.clone()))
                }
                None => Ok(None),
            }
        }
    }

    fn service(
    ) -> PollsService<FakeRepo, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
    {
        PollsService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    #[tokio::test]
    async fn ask_add_choice_vote_and_read_back() {
        let svc = service();

        let q = svc
            .ask(CreateQuestion {
                text: "What's new?".into(),
            })
            .await
            .unwrap();
        assert_eq!(q.id, 1);

        let a = svc
            .add_choice(AddChoice {
                question_id: q.id,
                text: "Not much".into(),
            })
            .await
            .unwrap();
        let b = svc
            .add_choice(AddChoice {
                question_id: q.id,
                text: "The sky".into(),
            })
            .await
            .unwrap();
        assert_eq!((a.id, a.votes), (2, 0));
        assert_eq!((b.id, b.votes), (3, 0));

        // Two votes for the first choice.
        svc.vote(Vote { choice_id: a.id }).await.unwrap();
        let after = svc.vote(Vote { choice_id: a.id }).await.unwrap();
        assert_eq!(after.votes, 2);

        // The detail view reflects the tally.
        let detail = svc.detail(GetQuestion { id: q.id }).await.unwrap();
        assert_eq!(detail.question, q);
        assert_eq!(detail.choices.len(), 2);
        assert_eq!(detail.choices[0].votes, 2);

        // The index view lists the question.
        let list = svc.list().await.unwrap();
        assert_eq!(list, vec![q]);

        // Three writes committed (ask + two choices) plus two votes = 5.
        assert_eq!(svc.uow.outcomes().len(), 5);
        assert!(svc
            .uow
            .outcomes()
            .iter()
            .all(|o| *o == TxOutcome::Committed));
        assert_eq!(
            svc.events.topics(),
            [
                "polls.question_asked",
                "polls.choice_added",
                "polls.choice_added",
                "polls.voted",
                "polls.voted",
            ]
        );
    }

    #[tokio::test]
    async fn empty_question_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc.ask(CreateQuestion { text: "   ".into() }).await;
        assert!(err.is_err());
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_choice_needs_an_existing_question() {
        let svc = service();
        let err = svc
            .add_choice(AddChoice {
                question_id: 999,
                text: "orphan".into(),
            })
            .await;
        assert!(err.is_err());
        assert!(svc.uow.outcomes().is_empty());
    }

    #[tokio::test]
    async fn voting_an_unknown_choice_errors_and_commits_nothing() {
        let svc = service();
        let err = svc.vote(Vote { choice_id: 404 }).await;
        assert!(err.is_err());
        // The transaction opened but rolled back when the choice was missing.
        assert_eq!(svc.uow.outcomes(), [TxOutcome::RolledBack]);
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }
}
