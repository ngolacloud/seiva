//! Infrastructure layer of the `notifications` module: adapters that implement
//! the application's outbound ports.
//!
//! Depends on `application` + `domain` + `seiva-ports`. In-memory here so the
//! wiring is provable without a database; the real module is DORM-backed (and a
//! real adapter would also hand the message to an email/SMS gateway).

use shop_notifications_domain::{Notification, NotificationRepository, RepositoryError};
use std::sync::Mutex;

/// An in-memory [`NotificationRepository`]. Replace with the DORM-backed
/// adapter.
#[derive(Default)]
pub struct InMemoryNotificationRepository {
    sent: Mutex<Vec<Notification>>,
}

impl InMemoryNotificationRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every notification stored so far — for composition-level assertions.
    pub fn sent(&self) -> Vec<Notification> {
        self.sent.lock().expect("repository poisoned").clone()
    }
}

impl NotificationRepository for InMemoryNotificationRepository {
    async fn save_notification(&self, notification: &Notification) -> Result<(), RepositoryError> {
        self.sent
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .push(notification.clone());
        Ok(())
    }

    async fn list_notifications(&self) -> Result<Vec<Notification>, RepositoryError> {
        Ok(self
            .sent
            .lock()
            .map_err(|_| RepositoryError("repository lock poisoned".to_owned()))?
            .clone())
    }
}
