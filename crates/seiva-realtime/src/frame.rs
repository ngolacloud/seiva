//! The opaque payload the hub carries, the topic it is grouped under, and the id
//! of a live connection.
//!
//! The framework transports **frames, not meanings**: a [`Frame`] is bytes the
//! hub never parses. A product gives its own message type an encode/decode by
//! implementing [`RealtimeMessage`]; the mechanism-vs-policy line (ADR 0031)
//! lives exactly here.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;

/// An opaque real-time payload — the unit the [`Hub`](crate::Hub) fans out.
///
/// The hub never looks inside a frame: a text frame is UTF-8 a browser receives
/// as a WebSocket text message, a binary frame is arbitrary bytes. The product's
/// protocol (signalling, chat, presence deltas) is encoded into a frame by a
/// [`RealtimeMessage`] and decoded on the way back — the framework stays neutral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A UTF-8 text payload (delivered as a WebSocket text message / SSE data).
    Text(String),
    /// An arbitrary binary payload (delivered as a WebSocket binary message).
    Binary(Bytes),
}

impl Frame {
    /// A text frame.
    pub fn text(value: impl Into<String>) -> Self {
        Frame::Text(value.into())
    }

    /// A binary frame.
    pub fn binary(value: impl Into<Bytes>) -> Self {
        Frame::Binary(value.into())
    }

    /// The payload size in bytes — what the hub's `max_frame_size` bounds.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Frame::Text(s) => s.len(),
            Frame::Binary(b) => b.len(),
        }
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A product message that can be carried as an opaque [`Frame`].
///
/// The hub depends on this trait, never on the concrete message type, so
/// `offer/answer/ice` or any chat/presence schema stays the product's policy.
/// `encode` is infallible (a message always has a wire form); `decode` may fail
/// on a malformed frame from the network.
pub trait RealtimeMessage: Sized {
    /// Why an inbound frame could not be decoded into this message.
    type Error;

    /// The wire form of this message.
    fn encode(&self) -> Frame;

    /// Parse a frame received from a peer back into a message.
    ///
    /// # Errors
    ///
    /// Returns [`Self::Error`] when the frame is not a valid encoding.
    fn decode(frame: Frame) -> Result<Self, Self::Error>;
}

/// A fan-out topic: an interned, cheap-to-clone label a connection subscribes to
/// (e.g. a product's `room:{id}`).
///
/// Backed by an `Arc<str>` so cloning a topic for every subscriber in a hot
/// `publish` is a refcount bump, not a string copy.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Topic(Arc<str>);

impl Topic {
    /// Intern a topic label.
    #[must_use]
    pub fn new(label: impl AsRef<str>) -> Self {
        Topic(Arc::from(label.as_ref()))
    }

    /// The topic label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Topic({:?})", &*self.0)
    }
}

impl From<&str> for Topic {
    fn from(value: &str) -> Self {
        Topic::new(value)
    }
}

impl From<String> for Topic {
    fn from(value: String) -> Self {
        Topic(Arc::from(value.as_str()))
    }
}

impl From<&String> for Topic {
    fn from(value: &String) -> Self {
        Topic::new(value)
    }
}

/// The identity of a live connection within one process, handed out by
/// [`Hub::register`](crate::Hub::register). Monotonic and unique per hub.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(u64);

impl ConnectionId {
    pub(crate) fn new(raw: u64) -> Self {
        ConnectionId(raw)
    }

    /// The raw numeric id (for logging/metrics).
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn:{}", self.0)
    }
}

impl fmt::Debug for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConnectionId({})", self.0)
    }
}
