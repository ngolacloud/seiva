//! The registry + topic fan-out proofs, the per-process/per-tenant caps, and that
//! `publish` forwards to the multi-pod bridge while `deliver_local` does not.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use seiva_identity::Principal;
use seiva_ports::Scope;
use seiva_realtime::{Authenticated, BridgeError, BroadcastBackend, Frame, Hub, HubConfig, Topic};

fn identity(subject: &str, tenant: &str) -> Authenticated {
    Authenticated::new(
        Principal::in_tenant(subject, tenant, ["realtime.connect"]),
        Scope::tenant(tenant).expect("non-empty tenant"),
    )
}

#[tokio::test]
async fn fan_out_reaches_only_topic_subscribers() {
    let hub = Hub::new();

    let a = hub.register(identity("a", "acme")).expect("register a");
    let b = hub.register(identity("b", "acme")).expect("register b");
    let mut c = hub.register(identity("c", "acme")).expect("register c");

    let room = Topic::new("room:1");
    assert!(hub.subscribe(a.id, room.clone()));
    assert!(hub.subscribe(b.id, room.clone()));
    assert!(hub.subscribe(c.id, Topic::new("room:2")));

    assert_eq!(hub.connection_count(), 3);
    assert_eq!(hub.subscriber_count(&room), 2);

    let delivery = hub
        .publish(room.clone(), Frame::text("hello"))
        .await
        .expect("publish");
    assert_eq!(delivery.delivered, 2, "both room:1 subscribers get it");
    assert_eq!(delivery.shed, 0);

    // a and b received exactly the frame; c (room:2) got nothing.
    let mut a = a;
    let mut b = b;
    assert_eq!(a.rx.try_recv().expect("a got it"), Frame::text("hello"));
    assert_eq!(b.rx.try_recv().expect("b got it"), Frame::text("hello"));
    assert!(c.rx.try_recv().is_err(), "c is on another topic");
}

#[tokio::test]
async fn unsubscribe_and_deregister_clean_the_registry() {
    let hub = Hub::new();
    let a = hub.register(identity("a", "acme")).expect("register a");
    let room = Topic::new("room:1");
    hub.subscribe(a.id, room.clone());
    assert_eq!(hub.subscriber_count(&room), 1);
    assert_eq!(hub.topic_count(), 1);

    assert!(hub.unsubscribe(a.id, room.clone()));
    assert_eq!(hub.subscriber_count(&room), 0);
    assert_eq!(hub.topic_count(), 0, "an empty topic is reaped");

    // Re-subscribe, then deregister: the topic is cleaned without an explicit
    // unsubscribe, and the tenant counter returns to zero.
    hub.subscribe(a.id, room.clone());
    assert_eq!(hub.tenant_connection_count("acme"), 1);
    hub.deregister(a.id);
    assert_eq!(hub.connection_count(), 0);
    assert_eq!(hub.subscriber_count(&room), 0);
    assert_eq!(hub.tenant_connection_count("acme"), 0);

    // Deregister is idempotent.
    hub.deregister(a.id);
    // Subscribing an unknown connection is a no-op that reports failure.
    assert!(!hub.subscribe(a.id, room));
}

#[tokio::test]
async fn per_process_cap_refuses_the_next_upgrade() {
    let hub = Hub::with_config(HubConfig {
        max_connections: 2,
        ..HubConfig::default()
    });
    let _a = hub.register(identity("a", "acme")).expect("1st fits");
    let b = hub.register(identity("b", "acme")).expect("2nd fits");
    assert!(
        hub.register(identity("c", "acme")).is_err(),
        "3rd is over the per-process cap"
    );

    // Freeing a slot lets a new connection in: the permit is returned on drop.
    hub.deregister(b.id);
    assert!(hub.register(identity("c", "acme")).is_ok());
}

#[tokio::test]
async fn per_tenant_cap_isolates_a_noisy_tenant() {
    let hub = Hub::with_config(HubConfig {
        max_connections: 100,
        max_connections_per_tenant: 1,
        ..HubConfig::default()
    });
    let _a = hub
        .register(identity("a", "acme"))
        .expect("acme's first fits");
    assert!(
        hub.register(identity("b", "acme")).is_err(),
        "acme is at its per-tenant cap"
    );
    // A different tenant is unaffected: the cap is per tenant, not global.
    assert!(hub.register(identity("x", "globex")).is_ok());
}

#[tokio::test]
async fn publish_rejects_an_oversized_frame() {
    let hub = Hub::with_config(HubConfig {
        max_frame_size: 8,
        ..HubConfig::default()
    });
    let err = hub
        .publish(Topic::new("t"), Frame::text("far too long a frame"))
        .await
        .expect_err("over the frame-size bound");
    assert!(matches!(
        err,
        seiva_realtime::RealtimeError::FrameTooLarge { .. }
    ));
}

/// A bridge that records every frame it was asked to forward to other pods.
#[derive(Clone, Default)]
struct RecordingBridge {
    seen: Arc<Mutex<Vec<(String, Frame)>>>,
}

#[async_trait]
impl BroadcastBackend for RecordingBridge {
    async fn publish(&self, topic: &Topic, frame: &Frame) -> Result<(), BridgeError> {
        self.seen
            .lock()
            .unwrap()
            .push((topic.as_str().to_owned(), frame.clone()));
        Ok(())
    }
}

#[tokio::test]
async fn publish_forwards_to_the_bridge_but_deliver_local_does_not() {
    let bridge = RecordingBridge::default();
    let hub = Hub::with_bridge(HubConfig::default(), Arc::new(bridge.clone()));

    let a = hub.register(identity("a", "acme")).expect("register");
    let room = Topic::new("room:1");
    hub.subscribe(a.id, room.clone());

    hub.publish(room.clone(), Frame::text("via publish"))
        .await
        .expect("publish");
    assert_eq!(
        bridge.seen.lock().unwrap().len(),
        1,
        "publish fans out to the bridge for other pods"
    );

    // A frame arriving FROM another pod is delivered locally only — it must not be
    // echoed back onto the bridge (no cross-pod loop).
    let _ = hub.deliver_local(&room, &Frame::text("from pod B"));
    assert_eq!(
        bridge.seen.lock().unwrap().len(),
        1,
        "deliver_local does not re-publish to the bridge"
    );
}
