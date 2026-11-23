//! The axum WebSocket + SSE transport (behind the `ws` feature).
//!
//! Both helpers take an **already-[`Authenticated`]** identity: the caller runs
//! [`authorize_upgrade`](crate::authorize_upgrade) first and completes the
//! upgrade only on `Ok`, so authentication always precedes the upgrade (ADR
//! 0031). [`serve_ws`] wires a bidirectional socket (outbound fan-out ← the hub,
//! inbound frames → an optional product channel) with a ping/pong heartbeat, an
//! idle timeout, a frame-size bound, and slow-consumer shedding. [`serve_sse`] is
//! the cheaper server→client-only path for dashboards and notifications.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use base64::Engine as _;
use bytes::Bytes;
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;

use crate::auth::Authenticated;
use crate::{ConnectionId, Frame, Hub, RealtimeError, Topic};

/// Liveness tuning for a WebSocket connection.
#[derive(Clone, Copy, Debug)]
pub struct WsConfig {
    /// How often the server sends a ping.
    pub heartbeat: Duration,
    /// How long without any inbound frame (incl. the client's pong) before the
    /// connection is reaped as dead.
    pub idle_timeout: Duration,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(120),
        }
    }
}

/// A frame received from a peer, tagged with the connection it came in on, handed
/// to the product's inbound channel. The product decodes it with its own
/// [`RealtimeMessage`](crate::RealtimeMessage) — the framework stays neutral.
#[derive(Clone, Debug)]
pub struct Inbound {
    /// The connection the frame arrived on.
    pub connection: ConnectionId,
    /// The opaque frame.
    pub frame: Frame,
}

/// Complete a WebSocket upgrade for an already-authenticated caller.
///
/// Registers the connection (enforcing the hub's caps), subscribes it to
/// `subscriptions`, and returns the upgrade [`Response`]. Inbound frames from the
/// peer are forwarded to `inbound` (if provided) tagged with the connection id;
/// pass `None` for a server→client-only socket.
///
/// # Errors
///
/// Propagates [`Hub::register`]'s caps errors (so the caller maps them to
/// `503`), in which case the socket is **not** upgraded.
pub fn serve_ws(
    hub: Hub,
    upgrade: WebSocketUpgrade,
    identity: Authenticated,
    subscriptions: Vec<Topic>,
    inbound: Option<mpsc::Sender<Inbound>>,
    config: WsConfig,
) -> Result<Response, RealtimeError> {
    let sink = hub.register(identity)?;
    let id = sink.id;
    for topic in &subscriptions {
        hub.subscribe(id, topic.clone());
    }
    let max_frame = hub.config().max_frame_size;
    Ok(upgrade.on_upgrade(move |socket| pump(hub, id, socket, sink, inbound, config, max_frame)))
}

async fn pump(
    hub: Hub,
    id: ConnectionId,
    socket: WebSocket,
    sink: crate::ConnectionSink,
    inbound: Option<mpsc::Sender<Inbound>>,
    config: WsConfig,
    max_frame: usize,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut outbound = sink.rx;
    let shed = sink.shed;
    let mut heartbeat = tokio::time::interval(config.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = tokio::time::Instant::now();

    loop {
        tokio::select! {
            // Outbound: frames the hub fanned out to this connection.
            frame = outbound.recv() => {
                match frame {
                    Some(frame) => {
                        if sender.send(to_message(frame)).await.is_err() {
                            break;
                        }
                    }
                    // The hub deregistered us (its sender dropped): drain done, close.
                    None => break,
                }
            }
            // The hub asked us to shed this connection NOW (slow consumer).
            () = shed.notified() => {
                let _ = sender.send(Message::Close(None)).await;
                break;
            }
            // Heartbeat tick: reap on idle, else ping.
            _ = heartbeat.tick() => {
                if last_seen.elapsed() >= config.idle_timeout {
                    break;
                }
                if sender.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            // Inbound: frames from the peer.
            msg = receiver.next() => {
                match msg {
                    Some(Ok(message)) => {
                        last_seen = tokio::time::Instant::now();
                        match message {
                            Message::Text(text) => {
                                if text.as_str().len() > max_frame {
                                    break;
                                }
                                forward_inbound(&inbound, id, Frame::Text(text.as_str().to_owned()));
                            }
                            Message::Binary(bytes) => {
                                if bytes.len() > max_frame {
                                    break;
                                }
                                forward_inbound(&inbound, id, Frame::Binary(bytes));
                            }
                            // axum answers a Ping with a Pong automatically; a Pong
                            // here just refreshed `last_seen` above.
                            Message::Ping(_) | Message::Pong(_) => {}
                            Message::Close(_) => break,
                        }
                    }
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    hub.deregister(id);
}

fn forward_inbound(channel: &Option<mpsc::Sender<Inbound>>, id: ConnectionId, frame: Frame) {
    if let Some(channel) = channel {
        // Best-effort: if the product's inbound queue is full, drop rather than
        // stall the socket pump (the product sizes its own queue).
        let _ = channel.try_send(Inbound {
            connection: id,
            frame,
        });
    }
}

fn to_message(frame: Frame) -> Message {
    match frame {
        Frame::Text(text) => Message::Text(text.into()),
        Frame::Binary(bytes) => Message::Binary(bytes),
    }
}

/// Serve a server→client-only **SSE** stream for an already-authenticated caller.
///
/// The cheaper, proxy-friendly, auto-reconnecting path (dashboards, notifications)
/// — there is no inbound direction. Text frames are sent verbatim; a binary frame
/// is base64-encoded for the text-only SSE channel. The connection is deregistered
/// automatically when the client disconnects (the stream is dropped).
///
/// # Errors
///
/// Propagates [`Hub::register`]'s caps errors (so the caller maps them to `503`).
pub fn serve_sse(
    hub: Hub,
    identity: Authenticated,
    subscriptions: Vec<Topic>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, RealtimeError> {
    let sink = hub.register(identity)?;
    let id = sink.id;
    for topic in &subscriptions {
        hub.subscribe(id, topic.clone());
    }

    // The guard rides inside the stream's state, so when the client disconnects
    // and the stream is dropped the connection is deregistered.
    let guard = DeregisterGuard { hub, id };
    let stream = futures_util::stream::unfold((sink.rx, guard), |(mut rx, guard)| async move {
        rx.recv()
            .await
            .map(|frame| (frame_to_event(frame), (rx, guard)))
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn frame_to_event(frame: Frame) -> Result<Event, Infallible> {
    let data = match frame {
        Frame::Text(text) => text,
        Frame::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes),
    };
    Ok(Event::default().data(data))
}

/// Deregisters an SSE connection when its stream is dropped (client disconnect).
struct DeregisterGuard {
    hub: Hub,
    id: ConnectionId,
}

impl Drop for DeregisterGuard {
    fn drop(&mut self) {
        self.hub.deregister(self.id);
    }
}
