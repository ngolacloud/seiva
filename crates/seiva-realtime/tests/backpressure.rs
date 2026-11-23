//! Per-connection backpressure: a consumer that fills its bounded send queue is
//! shed (closed), never buffered without bound — the slow-consumer policy of
//! ADR 0031.

use std::time::Duration;

use seiva_identity::Principal;
use seiva_ports::Scope;
use seiva_realtime::{Authenticated, Frame, Hub, HubConfig, Topic};

fn identity() -> Authenticated {
    Authenticated::new(
        Principal::in_tenant("slow", "acme", ["realtime.connect"]),
        Scope::tenant("acme").unwrap(),
    )
}

#[tokio::test]
async fn a_slow_consumer_is_shed_not_buffered() {
    // A one-slot send queue makes the second undrained frame overflow.
    let hub = Hub::with_config(HubConfig {
        send_buffer: 1,
        ..HubConfig::default()
    });

    // Keep the sink alive (so the channel stays OPEN) but never drain `rx`.
    let sink = hub.register(identity()).expect("register");
    let room = Topic::new("room:1");
    hub.subscribe(sink.id, room.clone());

    // First frame fits in the one-slot buffer.
    let first = hub
        .publish(room.clone(), Frame::text("1"))
        .await
        .expect("publish 1");
    assert_eq!(first.delivered, 1);
    assert_eq!(first.shed, 0);
    assert_eq!(hub.connection_count(), 1);

    // Second frame cannot be enqueued: the connection is shed, not buffered.
    let second = hub
        .publish(room.clone(), Frame::text("2"))
        .await
        .expect("publish 2");
    assert_eq!(second.delivered, 0);
    assert_eq!(second.shed, 1, "the slow consumer is shed");
    assert_eq!(hub.connection_count(), 0, "and removed from the registry");
    assert_eq!(hub.subscriber_count(&room), 0);

    // The transport is told to close the socket now (the shed signal fired).
    tokio::time::timeout(Duration::from_secs(1), sink.shed.notified())
        .await
        .expect("the shed signal must have been raised");
}

#[tokio::test]
async fn a_gone_consumer_is_shed_on_next_publish() {
    let hub = Hub::new();
    let sink = hub.register(identity()).expect("register");
    let room = Topic::new("room:1");
    hub.subscribe(sink.id, room.clone());
    assert_eq!(hub.connection_count(), 1);

    // The transport dropped its receiver (socket closed): the channel is now
    // closed, so the next publish sheds the dead connection.
    drop(sink);
    let d = hub
        .publish(room.clone(), Frame::text("x"))
        .await
        .expect("publish");
    assert_eq!(d.delivered, 0);
    assert_eq!(d.shed, 1);
    assert_eq!(hub.connection_count(), 0);
}
