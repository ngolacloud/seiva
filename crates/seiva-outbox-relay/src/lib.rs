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

use async_trait::async_trait;
use delonix_seiva::worker::{run_worker, WorkerConfig, WorkerReport};
use dorm_core::value::Value;
use dorm_core::{db, Error, Result};
use dorm_outbox::{RelayConfig, CURSOR, OUTBOX};
use seiva_events::{OutboxReader, OutboxRecord};
use seiva_messaging::{CloudEvent, Publisher};
use seiva_ports::{BoxError, PortResult};

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

/// Box a concrete error into the ports' opaque [`BoxError`], so a DORM/serde/
/// messaging failure crosses the [`OutboxReader`] boundary as the one uniform
/// failure type the application layer sees (`seiva-ports`).
fn boxed<E>(e: E) -> BoxError
where
    E: std::error::Error + Send + Sync + 'static,
{
    Box::new(e)
}

/// The DORM adapter of [`seiva_events::OutboxReader`]: the pull-based read side of
/// the **real** in-database outbox (`dorm-outbox`), so the DORM-free in-process
/// driver [`seiva_events::drain_in_process`] runs a monolith's choreography off the
/// same outbox a split service would relay to a broker (ADR 0021 Fase 2).
///
/// It reuses `dorm_outbox`'s own cursor semantics — read the events for a topic
/// after the per-consumer cursor (`id > position`, oldest first), and advance that
/// cursor forward only (never backwards) — but exposes them as the
/// [`read_batch`](OutboxReader::read_batch) / [`advance`](OutboxReader::advance) pair
/// the driver controls for at-least-once delivery, rather than the push-shaped
/// [`dorm_outbox::relay`] used by [`forward`]/[`drain`]. The event body is the same
/// CloudEvents envelope the broker path carries, so a subscriber handler is
/// byte-identical across the two paths.
///
/// Like every `dorm_core::db` call, [`read_batch`](OutboxReader::read_batch) and
/// [`advance`](OutboxReader::advance) run in the caller's ambient DORM session
/// (`with_session`); the driver, dispatch and inbox claim share it.
pub struct DormOutboxReader {
    consumer: String,
    topic: String,
}

impl DormOutboxReader {
    /// A reader for `consumer`'s cursor over `topic`. The pair keys the cursor row
    /// (`dorm_outbox`'s `CURSOR` table), so two consumers of the same topic each keep
    /// their own position (fan-out), exactly as the broker relay does.
    pub fn new(consumer: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            consumer: consumer.into(),
            topic: topic.into(),
        }
    }

    /// Ensure the cursor row exists and read its position — the same two statements
    /// `dorm_outbox::relay` runs before a drain pass.
    async fn position(&self) -> PortResult<i64> {
        db::run(
            &format!(
                "INSERT INTO {CURSOR} (consumer, topic) VALUES ($1, $2) ON CONFLICT DO NOTHING"
            ),
            &[
                Value::Text(self.consumer.clone()),
                Value::Text(self.topic.clone()),
            ],
        )
        .await
        .map_err(boxed)?;
        let rows = db::sql(
            &format!("SELECT position FROM {CURSOR} WHERE consumer = $1 AND topic = $2"),
            &[
                Value::Text(self.consumer.clone()),
                Value::Text(self.topic.clone()),
            ],
        )
        .await
        .map_err(boxed)?;
        rows.first()
            .ok_or_else(|| BoxError::from("the outbox cursor row vanished"))?
            .try_get::<i64>(0)
            .map_err(boxed)
    }
}

#[async_trait]
impl OutboxReader for DormOutboxReader {
    async fn read_batch(&self, max: usize) -> PortResult<Vec<OutboxRecord>> {
        let pos = self.position().await?;
        let rows = db::sql(
            &format!(
                "SELECT id, payload::text FROM {OUTBOX} \
                 WHERE topic = $1 AND id > $2 ORDER BY id LIMIT $3"
            ),
            &[
                Value::Text(self.topic.clone()),
                Value::I64(pos),
                Value::I64(max as i64),
            ],
        )
        .await
        .map_err(boxed)?;

        let mut batch = Vec::with_capacity(rows.len());
        for row in &rows {
            let id = row.try_get::<i64>(0).map_err(boxed)?;
            let payload_text = row.try_get::<String>(1).map_err(boxed)?;
            let payload: serde_json::Value = serde_json::from_str(&payload_text).map_err(boxed)?;
            let event = CloudEvent::from_json(&payload).map_err(boxed)?;
            batch.push(OutboxRecord {
                // Row ids are positive, so the cursor token fits u64 unchanged.
                position: id as u64,
                event,
            });
        }
        Ok(batch)
    }

    async fn advance(&self, position: u64) -> PortResult<()> {
        // Never backwards (`position < $3`), so a slower concurrent pass cannot
        // rewind the cursor — the same guard `dorm_outbox` keeps.
        db::run(
            &format!(
                "UPDATE {CURSOR} SET position = $3, updated_at = now() \
                 WHERE consumer = $1 AND topic = $2 AND position < $3"
            ),
            &[
                Value::Text(self.consumer.clone()),
                Value::Text(self.topic.clone()),
                Value::I64(position as i64),
            ],
        )
        .await
        .map_err(boxed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use seiva_events::{
        drain_in_process, run_in_process_dispatch_worker, EventSubscriber, SubscriberRegistry,
        WorkerConfig,
    };
    use seiva_testing::InMemoryIdempotencyStore;

    /// A test subscriber that "reserves stock" by counting its runs.
    struct Counting(Arc<AtomicUsize>);

    #[async_trait]
    impl EventSubscriber for Counting {
        fn subscriber_id(&self) -> &str {
            "inventory"
        }

        async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Live proof that [`drain_in_process`] drives the choreography off the **real**
    /// `dorm-outbox` through [`DormOutboxReader`]: an `orders.placed` published into
    /// the transactional outbox is drained exactly once, and a second pass drains
    /// nothing (the cursor advanced). Gated on `SEIVA_DORM_TEST_DSN` (a libpq DSN);
    /// it skips cleanly when unset, so the default `cargo test` stays offline — and
    /// this crate has no PostgreSQL in CI without a token.
    #[tokio::test]
    async fn live_drain_in_process_over_the_dorm_outbox() {
        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live outbox drain proof");
            return;
        };
        let db = Arc::new(
            dorm_core::db::Database::connect(&dsn, None, false)
                .expect("connect to the test database"),
        );
        dorm_core::db::with_session(db, async move {
            // A fresh outbox schema (idempotent DDL).
            for ddl in dorm_outbox::migrations() {
                dorm_core::db::run_ddl(&ddl)
                    .await
                    .expect("outbox migrations apply");
            }

            let topic = "orders.placed";
            let consumer = "inventory";
            let event = CloudEvent::new(
                "live-order-1",
                "/shop/orders",
                topic,
                serde_json::json!({ "order_id": "live-order-1" }),
            );

            // The orders module publishes inside a transaction (the outbox invariant:
            // the event commits with the business write).
            db::atomic(move |_tx| async move {
                dorm_outbox::publish(topic, &event.to_json()).await?;
                Ok(())
            })
            .await
            .expect("publish into the outbox");

            // Drive the DORM-free driver over the real outbox cursor.
            let reader = DormOutboxReader::new(consumer, topic);
            let runs = Arc::new(AtomicUsize::new(0));
            let mut registry = SubscriberRegistry::new();
            registry.register(topic, Arc::new(Counting(runs.clone())));
            let inbox = InMemoryIdempotencyStore::new();

            let n = drain_in_process(&reader, &registry, &inbox)
                .await
                .expect("the first drain pass succeeds");
            assert_eq!(n, 1, "one event drained");
            assert_eq!(runs.load(Ordering::SeqCst), 1, "inventory reserved once");

            let n = drain_in_process(&reader, &registry, &inbox)
                .await
                .expect("the second drain pass succeeds");
            assert_eq!(n, 0, "the cursor advanced; nothing left to drain");
            assert_eq!(
                runs.load(Ordering::SeqCst),
                1,
                "still exactly one reservation"
            );
        })
        .await;
    }

    /// The in-process **worker** (`run_in_process_dispatch_worker`) drains the real
    /// DORM outbox in a loop and stops on shutdown — proving a monolith progresses
    /// the choreography on its own, not just a single hand-driven `drain_in_process`
    /// pass. Uses a dedicated topic so the `(consumer, topic)` cursor is isolated
    /// from the other live test on a shared database. Gated on `SEIVA_DORM_TEST_DSN`.
    #[tokio::test]
    async fn live_in_process_worker_drains_the_dorm_outbox() {
        let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
            eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the live worker proof");
            return;
        };
        let db = Arc::new(
            dorm_core::db::Database::connect(&dsn, None, false)
                .expect("connect to the test database"),
        );
        dorm_core::db::with_session(db, async move {
            for ddl in dorm_outbox::migrations() {
                dorm_core::db::run_ddl(&ddl)
                    .await
                    .expect("outbox migrations apply");
            }

            // A topic dedicated to this test isolates its cursor and its rows from
            // the single-pass live test above (same table, same database).
            let topic = "orders.worker.placed";
            let consumer = "inventory-worker";

            // Two orders published transactionally into the real outbox.
            for id in ["w-order-1", "w-order-2"] {
                let event = CloudEvent::new(
                    id,
                    "/shop/orders",
                    topic,
                    serde_json::json!({ "order_id": id }),
                );
                db::atomic(move |_tx| async move {
                    dorm_outbox::publish(topic, &event.to_json()).await?;
                    Ok(())
                })
                .await
                .expect("publish into the outbox");
            }

            let reader = DormOutboxReader::new(consumer, topic);
            let runs = Arc::new(AtomicUsize::new(0));
            let mut registry = SubscriberRegistry::new();
            registry.register(topic, Arc::new(Counting(runs.clone())));
            let inbox = InMemoryIdempotencyStore::new();

            // Stop once both freshly-published orders have been reserved: the worker
            // drains them in its first pass, so this is already satisfied when the
            // shutdown is first polled, and the loop exits cleanly.
            let probe = runs.clone();
            let shutdown = async move {
                while probe.load(Ordering::SeqCst) < 2 {
                    tokio::task::yield_now().await;
                }
            };

            let report = run_in_process_dispatch_worker(
                WorkerConfig::new("live-in-process-dispatch", std::time::Duration::ZERO),
                shutdown,
                reader,
                registry,
                inbox,
            )
            .await;

            assert!(
                report.processed >= 2,
                "the worker drained at least the two published orders"
            );
            assert_eq!(
                runs.load(Ordering::SeqCst),
                2,
                "both orders were reserved exactly once"
            );
        })
        .await;
    }
}
