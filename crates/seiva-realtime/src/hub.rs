//! The connection registry and fan-out core.
//!
//! The [`Hub`] is a sharded ([`DashMap`]) registry of live connections grouped by
//! [`Topic`]. [`register`](Hub::register) admits a connection under the caps,
//! handing back a [`ConnectionSink`] the transport pumps to the socket;
//! [`subscribe`](Hub::subscribe)/[`unsubscribe`](Hub::unsubscribe) move a
//! connection in and out of a topic; [`publish`](Hub::publish) delivers a frame
//! to every local subscriber **and** hands it to the bridge for other pods.
//!
//! Backpressure is per connection: each socket owns a **bounded** queue, and a
//! consumer that fills it is **shed** (closed), never buffered without bound —
//! one stalled client cannot grow the process's memory (ADR 0031).

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use tokio::sync::{mpsc, Notify, OwnedSemaphorePermit, Semaphore};

use crate::auth::Authenticated;
use crate::bridge::{BroadcastBackend, NoBridge};
use crate::{ConnectionId, Frame, RealtimeError, Topic};

/// The sentinel tenant bucket for a [`Scope::Global`](seiva_ports::Scope::Global)
/// connection, kept out of any real tenant's namespace by a non-identifier prefix.
const GLOBAL_TENANT_KEY: &str = "\0global";

/// Resource limits for a [`Hub`]. A persistent connection is a different resource
/// than a request (ADR 0026/0031): these bound it over its whole life.
#[derive(Clone, Copy, Debug)]
pub struct HubConfig {
    /// The per-process connection cap: once reached, a new upgrade is refused
    /// with [`RealtimeError::TooManyConnections`] (the long-lived analogue of
    /// ADR 0026's request semaphore).
    pub max_connections: usize,
    /// The per-tenant connection cap: one tenant cannot exhaust the process.
    pub max_connections_per_tenant: usize,
    /// The bounded send-queue depth per connection. When a slow consumer fills
    /// it, the connection is shed rather than buffered unbounded.
    pub send_buffer: usize,
    /// The largest frame [`publish`](Hub::publish) accepts, in bytes.
    pub max_frame_size: usize,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            max_connections: 10_000,
            max_connections_per_tenant: 1_000,
            send_buffer: 64,
            max_frame_size: 1 << 20, // 1 MiB
        }
    }
}

/// What one [`publish`](Hub::publish) did locally: how many connections received
/// the frame and how many were shed as slow consumers in the process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delivery {
    /// Connections the frame was enqueued to.
    pub delivered: usize,
    /// Connections shed (closed) because their send queue was full or gone.
    pub shed: usize,
}

/// The transport's half of a connection: the outbound frame stream to pump to
/// the socket, plus the shed signal that asks the transport to close now.
///
/// Returned by [`Hub::register`]. The transport owns this; the hub keeps the
/// matching sender and shed handle in its registry.
pub struct ConnectionSink {
    /// The connection's id in the hub.
    pub id: ConnectionId,
    /// Outbound frames to write to the socket. Yields `None` when the hub has
    /// deregistered the connection (drain and close).
    pub rx: mpsc::Receiver<Frame>,
    /// Notified when the hub sheds this connection: the transport must close the
    /// socket immediately (slow-consumer shedding).
    pub shed: Arc<Notify>,
}

/// One live connection as the hub holds it.
struct ConnEntry {
    tx: mpsc::Sender<Frame>,
    shed: Arc<Notify>,
    /// The topics this connection is in, so deregistration cleans the reverse map.
    topics: Mutex<HashSet<Topic>>,
    /// The tenant bucket this connection counts against.
    tenant_key: String,
    /// The global connection permit, released when the entry is dropped.
    _permit: OwnedSemaphorePermit,
}

struct Inner {
    config: HubConfig,
    connections: DashMap<ConnectionId, ConnEntry>,
    /// topic → the ids subscribed to it.
    topics: DashMap<Topic, HashSet<ConnectionId>>,
    /// tenant bucket → live connection count, for the per-tenant cap.
    tenants: DashMap<String, usize>,
    bridge: Arc<dyn BroadcastBackend>,
    permits: Arc<Semaphore>,
    next_id: AtomicU64,
}

/// The connection registry and topic fan-out core (ADR 0031). Cheap to clone —
/// every clone shares one registry.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    /// A local-only hub with [`HubConfig::default`] limits and no bridge.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(HubConfig::default())
    }

    /// A local-only hub with explicit limits.
    #[must_use]
    pub fn with_config(config: HubConfig) -> Self {
        Self::build(config, Arc::new(NoBridge))
    }

    /// A multi-pod hub: `bridge` fans a published frame out to the other pods.
    #[must_use]
    pub fn with_bridge(config: HubConfig, bridge: Arc<dyn BroadcastBackend>) -> Self {
        Self::build(config, bridge)
    }

    fn build(config: HubConfig, bridge: Arc<dyn BroadcastBackend>) -> Self {
        let permits = Arc::new(Semaphore::new(config.max_connections));
        Self {
            inner: Arc::new(Inner {
                config,
                connections: DashMap::new(),
                topics: DashMap::new(),
                tenants: DashMap::new(),
                bridge,
                permits,
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// The hub's limits.
    #[must_use]
    pub fn config(&self) -> &HubConfig {
        &self.inner.config
    }

    /// Admit a connection under the caps, binding it to `identity`.
    ///
    /// Returns the [`ConnectionSink`] the transport pumps to the socket. The
    /// connection is registered but subscribed to nothing yet — call
    /// [`subscribe`](Hub::subscribe) for each topic it should receive.
    ///
    /// # Errors
    ///
    /// - [`RealtimeError::TooManyConnections`] if the per-process cap is reached;
    /// - [`RealtimeError::TenantQuotaExceeded`] if the caller's tenant is at its cap.
    pub fn register(&self, identity: Authenticated) -> Result<ConnectionSink, RealtimeError> {
        // The global cap first: the permit is released automatically if we bail
        // out below (it is dropped on the early return) or when the entry drops.
        let permit = Arc::clone(&self.inner.permits)
            .try_acquire_owned()
            .map_err(|_| RealtimeError::TooManyConnections)?;

        let tenant_key = tenant_key(&identity.scope);

        // The per-tenant cap: check-and-increment under the shard's write guard so
        // two concurrent upgrades for one tenant cannot both slip past the cap.
        {
            let mut slot = self.inner.tenants.entry(tenant_key.clone()).or_insert(0);
            if *slot >= self.inner.config.max_connections_per_tenant {
                return Err(RealtimeError::TenantQuotaExceeded);
            }
            *slot += 1;
        }

        // From here on registration is infallible, so the reserved tenant slot and
        // the permit are never leaked.
        let id = ConnectionId::new(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel(self.inner.config.send_buffer.max(1));
        let shed = Arc::new(Notify::new());

        self.inner.connections.insert(
            id,
            ConnEntry {
                tx,
                shed: Arc::clone(&shed),
                topics: Mutex::new(HashSet::new()),
                tenant_key,
                _permit: permit,
            },
        );

        Ok(ConnectionSink { id, rx, shed })
    }

    /// Remove a connection: drop it from every topic, release its tenant slot and
    /// its global permit. Idempotent — deregistering an unknown id is a no-op.
    pub fn deregister(&self, id: ConnectionId) {
        let Some((_, entry)) = self.inner.connections.remove(&id) else {
            return;
        };
        let topics = {
            let guard = entry.topics.lock().expect("topics mutex poisoned");
            guard.iter().cloned().collect::<Vec<_>>()
        };
        for topic in topics {
            let now_empty = match self.inner.topics.get_mut(&topic) {
                Some(mut set) => {
                    set.remove(&id);
                    set.is_empty()
                }
                None => false,
            };
            if now_empty {
                // Drop the guard above before touching the same shard again.
                self.inner.topics.remove_if(&topic, |_, set| set.is_empty());
            }
        }
        if let Some(mut count) = self.inner.tenants.get_mut(&entry.tenant_key) {
            *count = count.saturating_sub(1);
        }
        self.inner
            .tenants
            .remove_if(&entry.tenant_key, |_, c| *c == 0);
        // `entry` drops here: its `_permit` returns to the semaphore and its `tx`
        // closes, so a transport still holding the `rx` sees `None` and closes.
    }

    /// Subscribe a registered connection to `topic`. Returns `false` if the id is
    /// unknown (e.g. already deregistered). Idempotent per (connection, topic).
    pub fn subscribe(&self, id: ConnectionId, topic: impl Into<Topic>) -> bool {
        let topic = topic.into();
        {
            let Some(entry) = self.inner.connections.get(&id) else {
                return false;
            };
            entry
                .topics
                .lock()
                .expect("topics mutex poisoned")
                .insert(topic.clone());
        }
        self.inner.topics.entry(topic).or_default().insert(id);
        true
    }

    /// Unsubscribe a connection from `topic`. Returns `false` if the id is unknown.
    pub fn unsubscribe(&self, id: ConnectionId, topic: impl Into<Topic>) -> bool {
        let topic = topic.into();
        {
            let Some(entry) = self.inner.connections.get(&id) else {
                return false;
            };
            entry
                .topics
                .lock()
                .expect("topics mutex poisoned")
                .remove(&topic);
        }
        let now_empty = match self.inner.topics.get_mut(&topic) {
            Some(mut set) => {
                set.remove(&id);
                set.is_empty()
            }
            None => false,
        };
        if now_empty {
            self.inner.topics.remove_if(&topic, |_, set| set.is_empty());
        }
        true
    }

    /// Publish `frame` to every local subscriber of `topic` **and** hand it to the
    /// bridge for the other pods.
    ///
    /// Local delivery is non-blocking: a connection whose bounded queue is full
    /// (or already gone) is **shed** — the slow-consumer policy of ADR 0031 — and
    /// counted in the returned [`Delivery`]. A bridge error is logged, not raised:
    /// local subscribers keep their frame.
    ///
    /// # Errors
    ///
    /// [`RealtimeError::FrameTooLarge`] if the frame exceeds `max_frame_size`.
    pub async fn publish(
        &self,
        topic: impl Into<Topic>,
        frame: Frame,
    ) -> Result<Delivery, RealtimeError> {
        let topic = topic.into();
        if frame.len() > self.inner.config.max_frame_size {
            return Err(RealtimeError::FrameTooLarge {
                size: frame.len(),
                max: self.inner.config.max_frame_size,
            });
        }
        let delivery = self.deliver_local(&topic, &frame);
        if let Err(e) = self.inner.bridge.publish(&topic, &frame).await {
            tracing::warn!(topic = %topic, error = %e, "realtime bridge publish failed");
        }
        Ok(delivery)
    }

    /// Deliver `frame` to the **local** subscribers of `topic` only, without
    /// touching the bridge. This is the entry point a bridge's receive loop uses
    /// for a frame arriving from another pod, so a remote frame is not echoed back
    /// onto the bridge (no cross-pod loop).
    #[must_use]
    pub fn deliver_local(&self, topic: &Topic, frame: &Frame) -> Delivery {
        // Snapshot the member ids and drop the topic shard guard before sending,
        // so a `try_send` (or a shed → deregister) never runs under a map lock.
        let members: Vec<ConnectionId> = match self.inner.topics.get(topic) {
            Some(set) => set.iter().copied().collect(),
            None => return Delivery::default(),
        };

        let mut delivered = 0;
        let mut to_shed = Vec::new();
        for id in members {
            // Snapshot the sender + shed handle, then release the shard guard.
            let handle = self
                .inner
                .connections
                .get(&id)
                .map(|e| (e.tx.clone(), Arc::clone(&e.shed)));
            let Some((tx, shed)) = handle else {
                continue;
            };
            match tx.try_send(frame.clone()) {
                Ok(()) => delivered += 1,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Slow consumer: close now, do not grow memory.
                    shed.notify_one();
                    to_shed.push(id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => to_shed.push(id),
            }
        }

        let shed = to_shed.len();
        for id in to_shed {
            self.deregister(id);
        }
        Delivery { delivered, shed }
    }

    /// The number of live connections (across all tenants and topics).
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.inner.connections.len()
    }

    /// The number of connections subscribed to `topic`.
    #[must_use]
    pub fn subscriber_count(&self, topic: &Topic) -> usize {
        self.inner.topics.get(topic).map_or(0, |set| set.len())
    }

    /// The number of distinct topics with at least one subscriber.
    #[must_use]
    pub fn topic_count(&self) -> usize {
        self.inner.topics.len()
    }

    /// The number of live connections for `tenant` (the per-tenant cap's counter).
    #[must_use]
    pub fn tenant_connection_count(&self, tenant: &str) -> usize {
        self.inner.tenants.get(tenant).map_or(0, |c| *c)
    }
}

/// The tenant bucket a scope counts against: the tenant id, or a reserved
/// sentinel for a global (cross-tenant) connection.
fn tenant_key(scope: &seiva_ports::Scope) -> String {
    scope
        .tenant_id()
        .map_or_else(|| GLOBAL_TENANT_KEY.to_owned(), |t| t.to_string())
}
