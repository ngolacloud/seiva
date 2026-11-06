//! Facade layer of the `notifications` module: the versioned public contract and
//! DTOs.
//!
//! The `notifications` module sends a message to a recipient and keeps a log of
//! what was sent. It is one of the five independent shop modules; it shares no
//! crate with the others — in the microservice story it reacts to their events
//! (an order placed, a payment captured) rather than calling into them.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireSchema};

/// Command DTO: send a notification to a recipient.
#[derive(Debug, Clone, WireSchema)]
pub struct SendNotification {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Response DTO for a notification. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, WireSchema)]
pub struct NotificationDto {
    pub id: u128,
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// The `notifications` module's public contract. Each method declares the
/// permission a caller must hold; the transport enforces it (deny by default,
/// ADR 0005).
#[facade(version = "v1")]
pub trait Notifications {
    /// Send a notification and return the recorded message.
    #[requires("notifications.send")]
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, String>;

    /// Every notification sent so far (the delivery log).
    #[requires("notifications.read")]
    async fn list(&self) -> Result<Vec<NotificationDto>, String>;
}
