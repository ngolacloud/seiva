//! The transactional-outbox → broker relay worker (Parte DORM-A; DORM ADR 0054).
//!
//! The old `dorm-messaging` `OutboxBus`, re-homed in Seiva. Consuming the
//! in-database outbox stays DORM's (`dorm-outbox`: `relay` one pass, `record_once`
//! for the inbox); this crate is the **relay loop** the brief assigns to the
//! framework — one pass drains a `dorm-outbox` batch and forwards each
//! [`CloudEvent`] through a `seiva-messaging` [`Publisher`], at-least-once. The
//! surrounding loop (interval, graceful shutdown, metrics, leader election) is
//! the caller's worker runtime.
//!
//! DORM is a pinned git dependency; this crate is excluded from the Seiva
//! workspace until the `dorm` feature wires it in (ADR 0004).

#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::Arc;

use delonix_seiva::worker::{run_worker, WorkerConfig, WorkerReport};
use dorm_core::{Error, Result};
use dorm_outbox::RelayConfig;
use seiva_messaging::{CloudEvent, Publisher};

/// Parse a [`CloudEvent`] from an outbox payload, mapping the messaging error to
/// a DORM error so the `dorm-outbox` relay can carry it.
fn cloud_event(payload: &serde_json::Value) -> Result<CloudEvent> {
    CloudEvent::from_json(payload).map_err(|e| Error::Config(e.to_string()))
}

/// Forward outbox events to an external broker through `publisher`, **one pass**,
/// at-least-once: drain what `consumer` has not seen for `topic` and publish each,
/// advancing the cursor only after a successful publish (a failed publish leaves
/// the cursor, so the event is retried; after `max_attempts` it dead-letters, as
/// [`dorm_outbox::relay`] defines). Returns how many it forwarded this pass.
pub async fn forward<P: Publisher>(
    consumer: &str,
    topic: &str,
    cfg: RelayConfig,
    publisher: &P,
) -> Result<u64> {
    let stats = dorm_outbox::relay(consumer, topic, cfg, move |ev| async move {
        let event = cloud_event(&ev.payload)?;
        publisher
            .publish(topic, &event)
            .await
            .map_err(|e| Error::Db(format!("broker publish failed: {e}")))
    })
    .await?;
    Ok(stats.processed)
}

/// Run the outbox → broker relay as a **worker**: the framework's loop
/// ([`run_worker`]) drives [`forward`] one pass at a time, with an interval,
/// graceful shutdown and metrics, until `shutdown` resolves. This is the whole
/// messaging path end to end — a business write lands an event in the outbox
/// transactionally (DORM), and this worker carries it to the broker,
/// at-least-once. Returns the accumulated [`WorkerReport`].
pub async fn run_relay_worker<P, S>(
    worker: WorkerConfig,
    shutdown: S,
    consumer: String,
    topic: String,
    relay_cfg: RelayConfig,
    publisher: P,
) -> WorkerReport
where
    P: Publisher,
    S: Future<Output = ()>,
{
    let publisher = Arc::new(publisher);
    run_worker(worker, shutdown, move || {
        // Each pass owns its clones, so the future borrows only its own locals.
        let publisher = publisher.clone();
        let consumer = consumer.clone();
        let topic = topic.clone();
        // RelayConfig is Copy — each pass gets its own copy.
        let cfg = relay_cfg;
        async move {
            forward(&consumer, &topic, cfg, &*publisher)
                .await
                // Bridge the DORM error to the worker's boundary error type.
                .map_err(|e| Box::new(e) as seiva_ports::BoxError)
        }
    })
    .await
}

/// Drain the outbox for `consumer`/`topic`, calling `handler` for each event
/// **exactly-once in effect**: the handler's database work and the inbox record
/// commit in one transaction, so a redelivery (at-least-once) is a no-op. One
/// pass; returns how many events were processed.
pub async fn drain<H, Fut>(consumer: &str, topic: &str, cfg: RelayConfig, handler: H) -> Result<u64>
where
    H: Fn(CloudEvent) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let handler = &handler;
    let consumer_owned = consumer.to_string();
    let stats = dorm_outbox::relay(consumer, topic, cfg, move |ev| {
        let consumer = consumer_owned.clone();
        async move {
            let event = cloud_event(&ev.payload)?;
            // The inbox guard and the handler's effect share one transaction:
            // record-once returns false on a redelivery, so the handler is
            // skipped — exactly-once effect under at-least-once delivery.
            dorm_core::db::atomic(move |_tx| async move {
                if dorm_outbox::record_once(&consumer, &event.id).await? {
                    handler(event).await
                } else {
                    Ok(())
                }
            })
            .await
        }
    })
    .await?;
    Ok(stats.processed)
}
