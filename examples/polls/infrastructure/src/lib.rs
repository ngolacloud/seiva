//! Infrastructure layer of the `polls` module: adapters that implement the
//! application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In a real module this is
//! the DORM-backed repository; here it is an in-memory adapter so the wiring is
//! provable without a database — the Seiva equivalent of running the Django
//! tutorial against SQLite before switching to Postgres.

use polls_domain::{Choice, PollRepository, Question, RepositoryError};
use std::sync::Mutex;

/// An in-memory [`PollRepository`]. Replace with the DORM-backed adapter.
#[derive(Default)]
pub struct InMemoryPollRepository {
    questions: Mutex<Vec<Question>>,
    choices: Mutex<Vec<Choice>>,
}

impl InMemoryPollRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every question stored so far — for composition-level assertions.
    pub fn questions(&self) -> Vec<Question> {
        self.questions.lock().expect("repository poisoned").clone()
    }

    /// Every choice stored so far — for composition-level assertions.
    pub fn choices(&self) -> Vec<Choice> {
        self.choices.lock().expect("repository poisoned").clone()
    }
}

impl PollRepository for InMemoryPollRepository {
    async fn save_question(&self, question: &Question) -> Result<(), RepositoryError> {
        self.questions
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(question.clone());
        Ok(())
    }

    async fn save_choice(&self, choice: &Choice) -> Result<(), RepositoryError> {
        self.choices
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(choice.clone());
        Ok(())
    }

    async fn list_questions(&self) -> Result<Vec<Question>, RepositoryError> {
        Ok(self
            .questions
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .clone())
    }

    async fn get_question(&self, id: u128) -> Result<Option<Question>, RepositoryError> {
        Ok(self
            .questions
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .iter()
            .find(|q| q.id == id)
            .cloned())
    }

    async fn choices_for(&self, question_id: u128) -> Result<Vec<Choice>, RepositoryError> {
        Ok(self
            .choices
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .iter()
            .filter(|c| c.question_id == question_id)
            .cloned()
            .collect())
    }

    async fn record_vote(&self, choice_id: u128) -> Result<Option<Choice>, RepositoryError> {
        let mut choices = self
            .choices
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?;
        match choices.iter_mut().find(|c| c.id == choice_id) {
            Some(choice) => {
                choice.cast_vote();
                Ok(Some(choice.clone()))
            }
            None => Ok(None),
        }
    }
}
