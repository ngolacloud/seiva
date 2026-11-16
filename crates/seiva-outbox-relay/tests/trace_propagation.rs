//! Proof of the event-boundary trace-context propagation on the **DORM relay
//! path** (ADR 0027, Fase 2): a choreography that crosses `outbox → drain →
//! handler` over the *real* transactional outbox is ONE distributed trace, with
//! the span that **emitted** the event as the parent of the span that **handles**
//! it — the DORM-backed twin of `crates/seiva-events/tests/trace_propagation.rs`
//! (which proves the same for the in-process driver).
//!
//! The test installs a real OpenTelemetry tracer with an in-memory span exporter
//! and the single W3C propagator the facade reads, then:
//!
//! 1. publishes `orders.trace.placed` **inside an emitter span** through the real
//!    [`DormOutboxPublisher`] — `inject_current` stamps the envelope's
//!    `traceparent` from that span and the event commits in the outbox
//!    transaction;
//! 2. drains the outbox through [`drain`], whose handling span (`seiva.outbox.drain`)
//!    is `set_parent`ed to the envelope's trace context;
//! 3. asserts the exported handling span shares the emitter's `trace_id` and carries
//!    the emitter's `span_id` as its `parent_span_id` — the emitter is its parent,
//!    so the trace survived the outbox→broker→subscriber hop.
//!
//! The facade's real runtime is on because this crate's dev-dependency enables
//! `seiva-observability/tracing`; a plain `cargo build -p seiva-outbox-relay` still
//! links nothing and both facade calls stay no-ops.
//!
//! Gated on `SEIVA_DORM_TEST_DSN` (a libpq DSN) like the crate's other live tests:
//! it skips cleanly when unset, so the token-gated `dorm-gateway` CI job (which has
//! no PostgreSQL) still compiles it under `--all-features` and is green, while a
//! real database proves the parentage.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing::Instrument;
use tracing_subscriber::prelude::*;

use dorm_core::db::{self, Database};
use dorm_outbox::RelayConfig;
use seiva_data_dorm::DormOutboxPublisher;
use seiva_outbox_relay::drain;
use seiva_ports::EventPublisher;

#[tokio::test]
async fn the_emitting_span_is_the_parent_of_the_relay_handling_span() {
    let Ok(dsn) = std::env::var("SEIVA_DORM_TEST_DSN") else {
        eprintln!("skipping: set SEIVA_DORM_TEST_DSN to run the relay trace-propagation proof");
        return;
    };

    // A real tracer with an in-memory exporter so we can inspect span parentage; a
    // simple processor exports each span synchronously on close.
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("seiva-outbox-relay-trace-test");

    // The single global W3C propagator the facade injects/extracts through — the
    // same type `seiva_observability::tracing::init` installs at a real root.
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Bridge `tracing` spans to the OTel tracer. `#[tokio::test]` runs on a
    // current-thread runtime, so this thread-local default subscriber is active
    // across every `.await` the choreography makes.
    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
    let _guard = tracing::subscriber::set_default(subscriber);

    let db = Arc::new(Database::connect(&dsn, None, false).expect("connect to the test database"));

    db::with_session(db, async move {
        // A fresh outbox schema (idempotent DDL).
        for ddl in dorm_outbox::migrations() {
            db::run_ddl(&ddl).await.expect("outbox migrations apply");
        }

        // A topic/consumer dedicated to this test isolates its cursor and rows from
        // the other live tests sharing this database.
        let topic = "orders.trace.placed";
        let consumer = "inventory-trace";

        // (1) Publish INSIDE an emitter span THROUGH the real DORM gateway publisher:
        //     `inject_current` stamps the envelope's `traceparent` from this span,
        //     and the event commits in the outbox transaction. The span closes when
        //     the future completes, so it is exported below.
        let payload = serde_json::to_vec(&serde_json::json!({ "order_id": "trace-1" })).unwrap();
        async move {
            db::atomic(move |_tx| async move {
                DormOutboxPublisher
                    .publish(topic, &payload)
                    .await
                    .map_err(|e| dorm_core::Error::Db(e.to_string()))?;
                Ok(())
            })
            .await
            .expect("publish into the outbox");
        }
        .instrument(tracing::info_span!("emitter.place_order"))
        .await;

        // (2) Drain → handle: `drain` opens a handling span parented to the
        //     envelope's trace context before running the handler.
        let handled = Arc::new(AtomicUsize::new(0));
        let probe = Arc::clone(&handled);
        let n = drain(consumer, topic, RelayConfig::default(), move |_event| {
            let probe = Arc::clone(&probe);
            async move {
                probe.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await
        .expect("the drain pass succeeds");
        assert_eq!(n, 1, "the one published event was relayed");
        assert_eq!(handled.load(Ordering::SeqCst), 1, "the handler ran once");

        // (3) Inspect the exported spans: the handler must join the emitter's trace
        //     with the emitter as its parent.
        provider.force_flush().expect("flush spans");
        let spans = exporter.get_finished_spans().expect("exported spans");

        let emitter = spans
            .iter()
            .find(|s| s.name == "emitter.place_order")
            .expect("the emitter span was exported");
        let handling = spans
            .iter()
            .find(|s| s.name == "seiva.outbox.drain")
            .expect("the handling span was exported");

        assert_eq!(
            handling.span_context.trace_id(),
            emitter.span_context.trace_id(),
            "the handler is in the SAME trace as the emitter — one distributed trace",
        );
        assert_eq!(
            handling.parent_span_id,
            emitter.span_context.span_id(),
            "the emitting span is the PARENT of the relay handling span",
        );
    })
    .await;
}
