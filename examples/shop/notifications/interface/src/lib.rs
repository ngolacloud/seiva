//! Interface layer of the `notifications` module: the transport-facing shell.
//!
//! Depends on the `facade` trait only; it holds any `Notifications`
//! implementation and exposes it. It never sees `domain` (ADR 0003). A
//! scaffolded module would let `#[seiva::controller]` generate the axum routes
//! from the facade; this example keeps a plain wrapper so the layering is
//! provable without the transport runtime.

use shop_notifications_facade::{NotificationDto, Notifications, SendNotification};

/// Wraps a `Notifications` implementation and exposes it.
pub struct NotificationsApi<S: Notifications> {
    pub service: S,
}

impl<S: Notifications> NotificationsApi<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }

    /// The handler a `POST /v1/notifications` route will call.
    pub async fn send(
        &self,
        to: String,
        subject: String,
        body: String,
    ) -> Result<NotificationDto, String> {
        self.service
            .send(SendNotification { to, subject, body })
            .await
    }

    /// The handler the `GET /v1/notifications` index route will call.
    pub async fn list(&self) -> Result<Vec<NotificationDto>, String> {
        self.service.list().await
    }
}
