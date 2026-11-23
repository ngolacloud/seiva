//! The multi-pod seam: a [`BroadcastBackend`] forwards a locally-published frame
//! to the **other** pods subscribed to the same topic.
//!
//! Without a bridge the hub is correct only within one process; horizontal
//! scaling would silently drop a frame published on pod A to a subscriber on pod
//! B (ADR 0031). The default is [`NoBridge`] (local-only, no cost); a deployment
//! running several replicas opts into a real adapter (e.g. the Redis pub/sub
//! bridge behind the `redis-bridge` feature). A bridge's receive loop delivers
//! remote frames into the local hub with
//! [`Hub::deliver_local`](crate::Hub::deliver_local) — never back through
//! [`publish`](crate::Hub::publish), so a frame is not echoed around the cluster.

use async_trait::async_trait;

use crate::{Frame, Topic};

/// Why a cross-pod broadcast failed. Transport-free so the trait drags no
/// backend type into callers; each adapter boxes its own error.
#[derive(Debug)]
pub enum BridgeError {
    /// The underlying backend (Redis/NATS/…) failed to accept the publish.
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Backend(e) => write!(f, "broadcast backend error: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BridgeError::Backend(e) => Some(&**e),
        }
    }
}

/// Forwards a locally-published frame to the other pods on the same topic.
///
/// Only the **send** side lives behind this trait; the receive side (subscribe
/// to the backend and call [`Hub::deliver_local`](crate::Hub::deliver_local)) is
/// an adapter-owned loop the composition root spawns, because its lifecycle is
/// the pod's, not a single publish's.
#[async_trait]
pub trait BroadcastBackend: Send + Sync {
    /// Publish `frame` on `topic` to every other pod.
    ///
    /// # Errors
    ///
    /// [`BridgeError`] if the backend rejects the publish. Local delivery has
    /// already happened by the time the hub calls this, so a bridge failure
    /// never costs the local subscribers their frame.
    async fn publish(&self, topic: &Topic, frame: &Frame) -> Result<(), BridgeError>;
}

/// The default local-only bridge: no cross-pod fan-out, no cost. A single-replica
/// service uses this and pays nothing for multi-pod machinery it does not need.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBridge;

#[async_trait]
impl BroadcastBackend for NoBridge {
    async fn publish(&self, _topic: &Topic, _frame: &Frame) -> Result<(), BridgeError> {
        Ok(())
    }
}
