# ADR 0027 — Native observability: Prometheus metrics and OpenTelemetry tracing as an opt-in framework primitive, with trace context carried across the event boundary

- Status: Proposed
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0012 (cloud-native health and limits), ADR 0021 (in-process event bus and choreography), ADR 0015 (distributed rate limit)

## Context

Seiva already does structured logging well: a JSON event stream to stdout, one
object per event, level via `RUST_LOG`, initialised before anything else so even a
config error is a parseable line (`crates/seiva-server/src/lib.rs`, `observability::init`).
That satisfies 12-Factor XI (logs as event streams). It is also the **only**
telemetry the framework emits.

Two gaps remain, and both bite precisely the deployment shape Seiva sells:

- **No metrics.** There is no `/metrics` endpoint, no Prometheus exporter, no
  `metrics` facade anywhere in `crates/`. The per-route load controls (ADR 0026),
  the rate limiter (ADR 0015), the authorization guard (ADR 0019) and the worker
  all take decisions — admit/shed/allow/deny, batches drained, lag — that an
  operator cannot see. The worker already computes a `WorkerReport` of counters and
  then **throws it away into a log line** (`crates/delonix-seiva/src/worker.rs`).
- **No distributed tracing.** There is no OpenTelemetry/OTLP exporter and no trace
  context propagation. This is tolerable in a monolith but **breaks the core value
  proposition**: the moment `seiva module split` turns a choreography
  (`orders.placed → inventory.reserved → payments.captured`) into separate
  services, a single logical operation becomes N disconnected log islands with no
  way to follow it across the HTTP hop *or* across the event/outbox hop.

The forces:

- **The core must stay DORM-free and runtime-light.** Observability cannot drag a
  heavy runtime into library crates that today link none. Like the rate limit
  (ADR 0015) and route controls (ADR 0026), the mechanism lives in the framework
  but the **policy and the exporters are wired at the composition root**, behind a
  cargo feature, so a build that does not opt in pays nothing.
- **Cardinality is a production hazard, not a detail.** A metric labelled by
  `tenant`, by raw request path, or by object id explodes the Prometheus series
  count and can take the scrape target down. The framework must emit
  **low-cardinality** series by construction (route *template*, status *class*,
  outcome) and must make the tenant a trace *attribute*, never a metric *label*.
- **Trace context must cross both boundaries Seiva owns.** The HTTP boundary (W3C
  `traceparent` in and out) *and* the asynchronous boundary — the CloudEvents
  envelope that carries a domain event through the transactional outbox, the
  broker and into a subscriber (ADR 0021, `seiva-messaging`). A trace that stops at
  the outbox is useless for the split-module case, which is the case that needs it.
- **It must compose with what exists.** The request-id correlation layer
  (`crates/seiva-http/src/request_id.rs`) is the natural place to also establish a
  span and extract/inject `traceparent`; the health probes must stay exempt from
  instrumentation noise exactly as they are exempt from the rate limiter.

## Decision

Introduce a new **workspace-member, DORM-free** crate `seiva-observability` and
wire it, opt-in, through `seiva-server` and `seiva-http`.

**Metrics (Prometheus).**

- Depend on the `metrics` facade crate (a thin, allocation-light recorder
  abstraction) in the library crates, and on `metrics-exporter-prometheus` only in
  `seiva-observability`/`seiva-server`. Library crates record against the facade;
  they never link an exporter.
- `seiva-http` records, per request: a request counter and a latency histogram
  keyed by **route template** (the matched rule, e.g. `/v1/projects/:id`, never the
  concrete path) and **status class** (`2xx`/`4xx`/`5xx`); an in-flight gauge; and
  the shed/deny decisions from the route controls (`503`/`504`), the rate limiter
  (`429`) and the guard (`403`). Tenant is **not** a label.
- The worker records its `WorkerReport` as counters/gauges (batches drained, items
  forwarded, failures, outbox lag) instead of only logging them.
- `seiva-server` exposes `/metrics` behind the `metrics` feature, served on a
  **separate bind** (an admin port, e.g. `SEIVA_METRICS_ADDR`) so the scrape
  surface is not the public API surface, and **exempt from auth and rate limiting**
  like the health probes.

**Tracing (OpenTelemetry/OTLP).**

- Behind a `tracing` (OTLP) feature, `seiva-observability` installs a
  `tracing-opentelemetry` layer with an OTLP exporter configured from the
  environment (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`, sampling via
  `OTEL_TRACES_SAMPLER*`). Without the feature, the existing JSON subscriber is
  unchanged.
- **HTTP boundary:** the request-id layer extracts the W3C `traceparent`/`tracestate`
  from the incoming request into the current span's context and injects the
  resulting context into the response, so an upstream trace continues through the
  service.
- **Event boundary:** the CloudEvents envelope in `seiva-messaging` gains a standard
  **trace-context extension** (`traceparent`/`tracestate` as envelope attributes).
  The publisher injects the current context when an event is written to the outbox;
  the in-process dispatcher and the broker-drain worker extract it when handling,
  so the span of the use case that *emitted* the event is the parent of the span
  that *handles* it — the trace survives the outbox→broker→subscriber hop and a
  `module split`.

**Policy at the root.** `seiva-server`'s `serve()` gains opt-in hooks
(`with_metrics(addr)`, `with_tracing(config)`); a service that calls neither emits
exactly today's logs and links no exporter. Health/readiness probes are exempt
from metrics and tracing noise, consistent with ADR 0012.

## Consequences

- An operator can finally see the decisions the framework already makes: shed/deny
  rates, per-route latency, in-flight concurrency, outbox lag — the signals needed
  to run the per-route controls (ADR 0026) and the distributed rate limit
  (ADR 0015) with real thresholds instead of guesses.
- The split-module story becomes operable: one `traceparent` follows a request
  through the HTTP hop and, crucially, through the event/outbox hop, so a
  choreography reads as one distributed trace whether it runs in a monolith or as
  N services. This closes the single biggest operational gap for using Seiva as a
  cloud platform's service layer.
- Cardinality is bounded by construction (route template + status class; tenant is
  a span attribute, not a metric label), so the scrape target stays healthy.
- The core stays DORM-free and zero-cost when unused: library crates record against
  the `metrics` facade (a no-op without a recorder) and the exporters/tracing live
  behind features wired only at the root.
- It does **not** replace a platform observability stack (collector, Prometheus,
  Tempo/Jaeger, Grafana): Seiva emits OTLP and a Prometheus scrape surface; the
  cluster operates the backends. This is the in-process instrumentation floor, not
  a monitoring product.

## Alternatives considered

- **`tracing` JSON logs only, correlate by request id (status quo).** Rejected as
  the whole decision: logs do not aggregate into latency/throughput series and a
  request id does not propagate across services, so the split-module case — the
  differentiator — has no distributed view.
- **Bake OpenTelemetry directly into the core crates, always on.** Rejected: it
  would link a heavy exporter and a runtime into libraries that today link none,
  violating the DORM-free/zero-cost-when-unused invariant the rate limit and route
  controls established. Feature-gated at the root keeps the default build lean.
- **Tenant as a metric label for per-tenant dashboards.** Rejected: unbounded
  cardinality is a self-inflicted outage. Per-tenant visibility belongs in traces
  (a span attribute) and in logs, not in the Prometheus series space.
- **A bespoke metrics format / endpoint.** Rejected: Prometheus scrape + OTLP are
  the cloud-native lingua franca; a custom format would need a custom collector and
  buy nothing.
