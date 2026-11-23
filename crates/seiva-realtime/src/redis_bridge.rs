//! A Redis pub/sub [`BroadcastBackend`] (behind the `redis-bridge` feature): the
//! multi-pod bridge that makes horizontal scaling correct instead of silently
//! lossy (ADR 0031).
//!
//! [`publish`](BroadcastBackend::publish) `PUBLISH`es a frame on a per-topic
//! channel; every pod's [`run`](RedisBridge::run) loop `PSUBSCRIBE`s the topic
//! namespace and delivers each arriving frame into its **local** hub with
//! [`Hub::deliver_local`](crate::Hub::deliver_local) — never back through
//! [`publish`](crate::Hub::publish), so a frame is not echoed around the cluster.
//! Same `redis` client as `seiva-ratelimit-redis` (proven in this workspace).

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use redis::aio::ConnectionManager;

use crate::bridge::{BridgeError, BroadcastBackend};
use crate::{Frame, Hub, Topic};

/// The default Redis channel prefix: topic `room:1` → channel `seiva:rt:room:1`.
const DEFAULT_PREFIX: &str = "seiva:rt:";

/// A Redis pub/sub multi-pod bridge. Cloning shares the pooled publish connection.
#[derive(Clone)]
pub struct RedisBridge {
    conn: ConnectionManager,
    client: redis::Client,
    prefix: String,
}

impl RedisBridge {
    /// Connect to Redis at `url` (e.g. `redis://127.0.0.1:6379`).
    ///
    /// # Errors
    ///
    /// [`BridgeError`] if the URL is invalid or the initial connection fails.
    pub async fn connect(url: &str) -> Result<Self, BridgeError> {
        let client = redis::Client::open(url).map_err(bridge_err)?;
        let conn = ConnectionManager::new(client.clone())
            .await
            .map_err(bridge_err)?;
        Ok(Self {
            conn,
            client,
            prefix: DEFAULT_PREFIX.to_owned(),
        })
    }

    /// Override the channel prefix (default `seiva:rt:`). Every pod sharing a topic
    /// space must use the same prefix.
    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// The receive loop: subscribe to the topic namespace and deliver every frame
    /// arriving from another pod into `hub` locally. Spawn this once per pod; it
    /// runs until the Redis connection drops.
    ///
    /// # Errors
    ///
    /// [`BridgeError`] if the subscription cannot be established.
    pub async fn run(&self, hub: Hub) -> Result<(), BridgeError> {
        let mut pubsub = self.client.get_async_pubsub().await.map_err(bridge_err)?;
        pubsub
            .psubscribe(format!("{}*", self.prefix))
            .await
            .map_err(bridge_err)?;
        let mut messages = pubsub.on_message();
        while let Some(msg) = messages.next().await {
            let channel = msg.get_channel_name().to_owned();
            let topic = channel
                .strip_prefix(self.prefix.as_str())
                .unwrap_or(&channel);
            let Ok(payload) = msg.get_payload::<Vec<u8>>() else {
                continue;
            };
            if let Some(frame) = decode_frame(&payload) {
                let _ = hub.deliver_local(&Topic::new(topic), &frame);
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BroadcastBackend for RedisBridge {
    async fn publish(&self, topic: &Topic, frame: &Frame) -> Result<(), BridgeError> {
        let channel = format!("{}{}", self.prefix, topic.as_str());
        let payload = encode_frame(frame);
        let mut conn = self.conn.clone();
        let _: () = redis::cmd("PUBLISH")
            .arg(channel)
            .arg(payload)
            .query_async(&mut conn)
            .await
            .map_err(bridge_err)?;
        Ok(())
    }
}

fn bridge_err(e: redis::RedisError) -> BridgeError {
    BridgeError::Backend(Box::new(e))
}

/// Wire format: a one-byte tag (`T`ext / `B`inary) then the raw payload.
fn encode_frame(frame: &Frame) -> Vec<u8> {
    match frame {
        Frame::Text(text) => {
            let mut buf = Vec::with_capacity(1 + text.len());
            buf.push(b'T');
            buf.extend_from_slice(text.as_bytes());
            buf
        }
        Frame::Binary(bytes) => {
            let mut buf = Vec::with_capacity(1 + bytes.len());
            buf.push(b'B');
            buf.extend_from_slice(bytes);
            buf
        }
    }
}

fn decode_frame(payload: &[u8]) -> Option<Frame> {
    let (tag, rest) = payload.split_first()?;
    match tag {
        b'T' => String::from_utf8(rest.to_vec()).ok().map(Frame::Text),
        b'B' => Some(Frame::Binary(Bytes::copy_from_slice(rest))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_frame, encode_frame};
    use crate::Frame;

    #[test]
    fn frame_round_trips_through_the_wire_format() {
        for frame in [
            Frame::text("offer:{...}"),
            Frame::binary(vec![0u8, 1, 2, 255]),
            Frame::text(""),
        ] {
            let bytes = encode_frame(&frame);
            assert_eq!(decode_frame(&bytes), Some(frame));
        }
        // A payload with an unknown tag or no tag decodes to nothing.
        assert_eq!(decode_frame(b"Zxyz"), None);
        assert_eq!(decode_frame(b""), None);
    }
}
