//! Proof of the event-boundary trace-context propagation (ADR 0027, Fase 2): a
//! choreography that crosses `outbox → drain → dispatch` is ONE distributed trace,
//! with the span that **emitted** the event as the parent of the span that
//! **handles** it.
//!
//! The test installs a real OpenTelemetry tracer with an in-memory span exporter and
//! the single W3C propagator the facade reads, then:
//!
//! 1. publishes `orders.placed` **inside an emitter span** — `inject_current` stamps
//!    the envelope's `traceparent` from that span;
//! 2. drains the outbox through [`drain_in_process`], whose [`dispatch`] opens a
//!    handling span and `set_parent`s it to the envelope's trace context;
//! 3. asserts the exported handling span shares the emitter's `trace_id` and carries
//!    the emitter's `span_id` as its `parent_span_id` — i.e. the emitter is its
//!    parent, so the trace survived the hop.
//!
//! The facade's real runtime is on because this crate's dev-dependency enables
//! `seiva-observability/tracing`; a plain `cargo build -p seiva-events` still links
//! nothing and both facade calls stay no-ops.
//!
//! [`drain_in_process`]: seiva_events::drain_in_process
//! [`dispatch`]: seiva_events::dispatch

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tracing::Instrument;
use tracing_subscriber::prelude::*;

use seiva_events::{
    drain_in_process, EventSubscriber, InMemoryOutbox, InMemoryOutboxPublisher, SubscriberRegistry,
};
use seiva_messaging::CloudEvent;
use seiva_ports::{EventPublisher, PortResult};
use seiva_testing::InMemoryIdempotencyStore;

/// A subscriber that records it was run — the observable effect we parent to the
/// emitter's trace.
struct RecordingSubscriber {
    id: String,
    handled: Arc<AtomicUsize>,
}

#[async_trait]
impl EventSubscriber for RecordingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }

    async fn handle(&self, _event: &CloudEvent) -> PortResult<()> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn the_emitting_span_is_the_parent_of_the_handling_span() {
    // A real tracer with an in-memory exporter so we can inspect span parentage; a
    // simple processor exports each span synchronously on close.
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("seiva-events-trace-test");

    // The single global W3C propagator the facade injects/extracts through — the
    // same type `seiva_observability::tracing::init` installs at a real root.
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Bridge `tracing` spans to the OTel tracer. `#[tokio::test]` runs on a
    // current-thread runtime, so this thread-local default subscriber is active
    // across every `.await` the choreography makes.
    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
    let _guard = tracing::subscriber::set_default(subscriber);

    // The offline choreography: a publisher writing the shared outbox, one
    // subscriber, and the in-process driver.
    let outbox = Arc::new(InMemoryOutbox::new());
    let publisher = InMemoryOutboxPublisher::new(Arc::clone(&outbox), "/shop/orders");
    let handled = Arc::new(AtomicUsize::new(0));
    let mut registry = SubscriberRegistry::new();
    registry.register(
        "orders.placed",
        Arc::new(RecordingSubscriber {
            id: "inventory".to_string(),
            handled: Arc::clone(&handled),
        }),
    );
    let inbox = InMemoryIdempotencyStore::new();

    // (1) Publish INSIDE an emitter span: `inject_current` stamps the envelope's
    //     `traceparent` from this span. The span closes when the future completes,
    //     so it is exported below.
    let payload = serde_json::to_vec(&serde_json::json!({ "order": "o-1" })).unwrap();
    async {
        publisher.publish("orders.placed", &payload).await.unwrap();
    }
    .instrument(tracing::info_span!("emitter.place_order"))
    .await;

    // (2) Drain → dispatch: the handling span is parented to the envelope's context.
    let dispatched = drain_in_process(&*outbox, &registry, &inbox)
        .await
        .expect("drain succeeds");
    assert_eq!(dispatched, 1, "the one published event was dispatched");
    assert_eq!(handled.load(Ordering::SeqCst), 1, "the subscriber ran once");

    // (3) Inspect the exported spans: the handler must join the emitter's trace with
    //     the emitter as its parent.
    provider.force_flush().expect("flush spans");
    let spans = exporter.get_finished_spans().expect("exported spans");

    let emitter = spans
        .iter()
        .find(|s| s.name == "emitter.place_order")
        .expect("the emitter span was exported");
    let handling = spans
        .iter()
        .find(|s| s.name == "seiva.event.dispatch")
        .expect("the handling span was exported");

    assert_eq!(
        handling.span_context.trace_id(),
        emitter.span_context.trace_id(),
        "the handler is in the SAME trace as the emitter — one distributed trace",
    );
    assert_eq!(
        handling.parent_span_id,
        emitter.span_context.span_id(),
        "the emitting span is the PARENT of the handling span",
    );
}
