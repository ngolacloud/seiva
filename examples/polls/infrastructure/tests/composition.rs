//! Composition-level test for the `polls` example: wire the `PollsService` over
//! the **real** in-memory infrastructure adapter ([`InMemoryPollRepository`]) and
//! the framework's test ports, then drive the whole module through its public
//! [`Polls`] facade — the same shape a composition root in `apps/` assembles.
//!
//! The application's own unit tests use a hand-rolled `FakeRepo`; this test
//! closes the loop by exercising the adapter this crate actually ships, so the
//! wiring (`infrastructure -> application -> facade`) is proven end to end.

use polls_application::PollsService;
use polls_facade::{AddChoice, CreateQuestion, GetQuestion, Polls, Vote};
use polls_infrastructure::InMemoryPollRepository;
use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};

fn service(
) -> PollsService<InMemoryPollRepository, SequenceIdGenerator, InMemoryUnitOfWork, RecordingEventPublisher>
{
    PollsService {
        repository: InMemoryPollRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    // Ask a question, add two choices — the Django tutorial's setup.
    let q = svc
        .ask(CreateQuestion {
            text: "What's up?".into(),
        })
        .await
        .expect("ask");
    let a = svc
        .add_choice(AddChoice {
            question_id: q.id,
            text: "Not much".into(),
        })
        .await
        .expect("add choice a");
    svc.add_choice(AddChoice {
        question_id: q.id,
        text: "The sky".into(),
    })
    .await
    .expect("add choice b");

    // Vote twice for the first choice; the tally reflects it.
    svc.vote(Vote { choice_id: a.id }).await.expect("vote 1");
    let after = svc.vote(Vote { choice_id: a.id }).await.expect("vote 2");
    assert_eq!(after.votes, 2);

    // The detail/results view reads back the real stored state.
    let detail = svc.detail(GetQuestion { id: q.id }).await.expect("detail");
    assert_eq!(detail.question, q);
    assert_eq!(detail.choices.len(), 2);
    let voted = detail
        .choices
        .iter()
        .find(|c| c.id == a.id)
        .expect("choice a present");
    assert_eq!(voted.votes, 2);

    // The index view lists the one question.
    assert_eq!(svc.list().await.expect("list"), vec![q]);

    // The adapter actually holds the rows (its own composition-level accessors).
    assert_eq!(svc.repository.questions().len(), 1);
    assert_eq!(svc.repository.choices().len(), 2);
}
