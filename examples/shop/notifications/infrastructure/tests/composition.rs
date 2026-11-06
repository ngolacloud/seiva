//! Composition-level test for the `notifications` example: wire the
//! `NotificationsService` over the **real** in-memory adapter
//! ([`InMemoryNotificationRepository`]) and the framework's test ports, then
//! drive the module through its public [`Notifications`] facade.

use seiva_testing::{InMemoryUnitOfWork, RecordingEventPublisher, SequenceIdGenerator};
use shop_notifications_application::NotificationsService;
use shop_notifications_facade::{Notifications, SendNotification};
use shop_notifications_infrastructure::InMemoryNotificationRepository;

fn service() -> NotificationsService<
    InMemoryNotificationRepository,
    SequenceIdGenerator,
    InMemoryUnitOfWork,
    RecordingEventPublisher,
> {
    NotificationsService {
        repository: InMemoryNotificationRepository::new(),
        ids: SequenceIdGenerator::new(),
        uow: InMemoryUnitOfWork::new(),
        events: RecordingEventPublisher::new(),
    }
}

#[tokio::test]
async fn full_flow_over_the_real_in_memory_adapter() {
    let svc = service();

    svc.send(SendNotification {
        to: "ada@example.com".into(),
        subject: "Order placed".into(),
        body: "Thanks for your order!".into(),
    })
    .await
    .expect("send");
    svc.send(SendNotification {
        to: "grace@example.com".into(),
        subject: "Payment captured".into(),
        body: "Your card was charged.".into(),
    })
    .await
    .expect("send");

    // The delivery log reads back the real stored state.
    assert_eq!(svc.list().await.expect("list").len(), 2);
    assert_eq!(svc.repository.sent().len(), 2);
}
