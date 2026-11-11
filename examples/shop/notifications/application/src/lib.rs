//! Application layer of the `notifications` module: the `NotificationsService`
//! (which implements the public facade) and the `NotificationRepository`
//! outbound port.
//!
//! Depends on `domain`, its own `facade` (DTOs + trait) and `seiva-ports`. It
//! names no `delonix_seiva::` item (ADR 0003) and consumes the framework's
//! cross-cutting ports (`UnitOfWork`, `IdGenerator`, `EventPublisher`) through
//! `seiva-ports`.

#![allow(async_fn_in_trait)]

use seiva_ports::{BoxError, EventPublisher, IdGenerator, PortResult, Scope, UnitOfWork};
// Classic DDD (ADR 0003): the repository PORT is defined in the DOMAIN. Re-exported
// here so `interface` imports it from `application` (never naming `domain`), keeping
// the layer rule intact; the application maps its domain error to `PortResult`.
use shop_notifications_domain::Notification;
pub use shop_notifications_domain::{NotificationRepository, RepositoryError};
use shop_notifications_facade::{
    NotificationDto, Notifications, NotificationsError, SendNotification,
};

/// The application service. Generic over the port and the framework's
/// cross-cutting ports, exactly like the other shop modules.
pub struct NotificationsService<R, G, U, E> {
    pub repository: R,
    pub ids: G,
    pub uow: U,
    pub events: E,
}

fn to_notification_dto(n: Notification) -> NotificationDto {
    NotificationDto {
        id: n.id.into(),
        to: n.to,
        subject: n.subject,
        body: n.body,
    }
}

impl<R, G, U, E> NotificationsService<R, G, U, E>
where
    R: NotificationRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn send_a_notification(&self, cmd: SendNotification) -> PortResult<NotificationDto> {
        let id = u128::from_be_bytes(self.ids.new_id().0);
        let notification =
            Notification::new(id, cmd.to, cmd.subject, cmd.body).map_err(BoxError::from)?;

        let event = serde_json::to_vec(&serde_json::json!({
            "id": notification.id.to_string(),
            "to": notification.to.clone(),
            "subject": notification.subject.clone(),
        }))
        .map_err(BoxError::from)?;

        // One transaction per use case (ADR 0004): the save AND the event publish
        // run together, so with the DORM gateway the event lands in the outbox in
        // the SAME transaction as the business change.
        self.uow
            .transaction(Scope::Global, || async {
                self.repository.save_notification(&notification).await?;
                self.events.publish("notifications.sent", &event).await
            })
            .await?;

        Ok(to_notification_dto(notification))
    }

    async fn all_notifications(&self) -> PortResult<Vec<NotificationDto>> {
        let notifications = self.repository.list_notifications().await?;
        Ok(notifications.into_iter().map(to_notification_dto).collect())
    }
}

impl<R, G, U, E> Notifications for NotificationsService<R, G, U, E>
where
    R: NotificationRepository + Sync,
    G: IdGenerator,
    U: UnitOfWork,
    E: EventPublisher,
{
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, NotificationsError> {
        self.send_a_notification(cmd)
            .await
            .map_err(|err| NotificationsError::Invalid(err.to_string()))
    }

    async fn list(&self) -> Result<Vec<NotificationDto>, NotificationsError> {
        self.all_notifications()
            .await
            .map_err(|err| NotificationsError::Invalid(err.to_string()))
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
        sent: Mutex<Vec<Notification>>,
    }

    impl NotificationRepository for FakeRepo {
        async fn save_notification(
            &self,
            notification: &Notification,
        ) -> Result<(), RepositoryError> {
            self.sent.lock().unwrap().push(notification.clone());
            Ok(())
        }

        async fn list_notifications(&self) -> Result<Vec<Notification>, RepositoryError> {
            Ok(self.sent.lock().unwrap().clone())
        }
    }

    fn service() -> NotificationsService<
        FakeRepo,
        SequenceIdGenerator,
        InMemoryUnitOfWork,
        RecordingEventPublisher,
    > {
        NotificationsService {
            repository: FakeRepo::default(),
            ids: SequenceIdGenerator::new(),
            uow: InMemoryUnitOfWork::new(),
            events: RecordingEventPublisher::new(),
        }
    }

    #[tokio::test]
    async fn send_and_list_a_notification() {
        let svc = service();

        let n = svc
            .send(SendNotification {
                to: "ada@example.com".into(),
                subject: "Order placed".into(),
                body: "Thanks for your order!".into(),
            })
            .await
            .unwrap();
        assert_eq!(n.id.to_string(), "1");
        assert_eq!(svc.list().await.unwrap(), vec![n]);

        assert_eq!(svc.uow.outcomes(), [TxOutcome::Committed]);
        assert_eq!(svc.events.topics(), ["notifications.sent"]);
    }

    #[tokio::test]
    async fn an_empty_recipient_is_rejected_by_the_domain_rule() {
        let svc = service();
        let err = svc
            .send(SendNotification {
                to: "  ".into(),
                subject: "Nope".into(),
                body: "Nobody".into(),
            })
            .await;
        assert!(err.is_err());
        assert!(svc.uow.outcomes().is_empty());
        assert_eq!(svc.events.topics(), Vec::<String>::new());
    }
}
