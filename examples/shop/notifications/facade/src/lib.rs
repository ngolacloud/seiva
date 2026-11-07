//! Facade layer of the `notifications` module: the versioned public contract and
//! DTOs.
//!
//! The `notifications` module sends a message to a recipient and keeps a log of
//! what was sent. It is one of the five independent shop modules; it shares no
//! crate with the others — in the microservice story it reacts to their events
//! (an order placed, a payment captured) rather than calling into them.
//!
//! The generated `#[seiva::controller]` (wired in `shop-notifications-interface`)
//! uses these DTOs as the wire types: the write DTO derives `serde` and
//! implements [`seiva_http::Validate`], the response DTO derives
//! [`serde::Serialize`], and the error maps to an RFC 9457 [`Problem`] here.

#![allow(async_fn_in_trait)]

use delonix_seiva::{facade, WireId, WireSchema};
use seiva_http::{Problem, Validate, ValidationError};
use serde::{Deserialize, Serialize};

/// Command DTO: send a notification to a recipient.
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct SendNotification {
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl Validate for SendNotification {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.to.trim().is_empty() {
            return Err(ValidationError::new(
                "notification recipient must not be empty",
            ));
        }
        Ok(())
    }
}

/// Response DTO for a notification. Never a domain type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, WireSchema)]
pub struct NotificationDto {
    pub id: WireId,
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// The `notifications` module's error type. A rule violation is a `422`, a
/// reference to a notification that does not exist a `404`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationsError {
    /// A domain rule was violated or the request could not be processed.
    Invalid(String),
    /// A referenced notification does not exist.
    NotFound(String),
}

impl std::fmt::Display for NotificationsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotificationsError::Invalid(m) | NotificationsError::NotFound(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for NotificationsError {}

impl From<NotificationsError> for Problem {
    fn from(error: NotificationsError) -> Self {
        match error {
            NotificationsError::Invalid(m) => Problem::unprocessable_entity(m),
            NotificationsError::NotFound(m) => Problem::not_found(m),
        }
    }
}

/// The `notifications` module's public contract. Each method declares the
/// permission a caller must hold; the transport enforces it (deny by default,
/// ADR 0005).
#[facade(version = "v1")]
pub trait Notifications {
    /// Send a notification and return the recorded message.
    #[requires("notifications.send")]
    async fn send(&self, cmd: SendNotification) -> Result<NotificationDto, NotificationsError>;

    /// Every notification sent so far (the delivery log).
    #[requires("notifications.read")]
    async fn list(&self) -> Result<Vec<NotificationDto>, NotificationsError>;
}
