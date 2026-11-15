# ADR 0026 — Per-route load control: a timeout (504) and concurrency backpressure (503) as a composable framework primitive

- Status: Accepted
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0015 (distributed rate limit), ADR 0019 (fail-closed authorization)

## Context

Seiva's HTTP transport already carries three OWASP API4 (Unrestricted Resource
Consumption) defences, each at a different granularity:

- a **body-size limit** bounds one request (`DefaultBodyLimit`),
- a **global request timeout** (30 s → `408`) stops a slowloris client pinning a
  worker on the whole service (`tower_http::timeout`),
- a **per-client rate limit** (→ `429`) bounds how fast one caller may issue
  requests, per process or shared across replicas via a store (ADR 0015).

None of them bounds the **work in flight on a single route**. A reporting endpoint
that takes seconds, or a write that stalls on a slow dependency, accumulates
requests until the pool is exhausted and *healthy* routes start failing too — the
classic one-slow-endpoint-takes-down-the-service failure. The global 30 s timeout
is far too coarse to protect a route whose own budget is 200 ms, and the rate
limit caps arrival *rate*, not *concurrency*: a burst within the window still all
runs at once.

The forces:

- The bound is a property of the **route**, not the service: `/v1/reports/export`
  may deserve 5 s and 4-in-flight while `/v1/projects` deserves 200 ms and 64. So
  the primitive must be **per route**, not one global knob.
- A saturated route must **fail fast** — shed the overflow immediately so latency
  stays bounded for admitted requests — rather than queue unboundedly (which just
  moves the collapse downstream).
- It must be **opt-in and composed at the root**, like the distributed rate limit
  (ADR 0015): the framework offers the mechanism, the composition root sets the
  policy. The framework core stays DORM-free and pulls in no runtime.
- The status codes must be **operator-honest**: a route deadline exceeded is a
  gateway timeout (`504`), a route at capacity is a transient unavailability
  (`503`) — distinct from the service-wide `408` and the per-client `429`.

## Decision

Add a first-class, opt-in primitive to `seiva-http`:

- **`RouteLimit`** — the limits for one route: an optional `timeout` (breach →
  `504 Gateway Timeout`) and an optional `max_in_flight` concurrency cap (breach →
  `503 Service Unavailable`). `max_in_flight(0)` is clamped to `1` so a typo never
  wedges a route shut.
- **`RouteControls`** — a set of `prefix → RouteLimit` rules; at request time the
  **longest** matching prefix wins, so a specific rule overrides a broad one. Each
  concurrency cap is materialised once into a shared `tokio::sync::Semaphore`, so
  all requests to the route draw from the SAME budget.
- **`with_route_controls(router, controls)`** — wraps a router in the enforcing
  middleware (an `axum` `from_fn` layer, matching the rate-limiter's style). An
  empty `RouteControls` adds no layer and costs nothing.

The middleware, per request: match the path to a rule; if a cap is set,
`try_acquire` a permit (**never queue** — shed with `503` on failure) and hold it
for the whole handling; then bound the handler with `tokio::time::timeout`,
returning `504` on breach. Both responses are RFC 9457 `Problem`s carrying the
request's correlation id; the `503` carries a short `Retry-After` (the breach is
transient — the permit frees the instant an in-flight request finishes).

**Composition.** The root applies `with_route_controls` to the **module**
sub-router *before* `router_with_modules` wraps the framework layers around it.
So control runs **inside** authentication (an admitted caller) but **around** each
route's own work — the correct order: authenticate, then bound the route.

`tokio` enters `seiva-http` as a `default-features = false` dependency with only
the `time` and `sync` leaves — no runtime is linked into the library.

## Consequences

- A slow or heavy route is now isolated: its timeout and concurrency cap keep it
  from exhausting the shared pool, and the rest of the service stays responsive.
- Per-route granularity means policy lives with the deployment, declaratively:
  `RouteControls::new().route("/v1/projects", RouteLimit::new().timeout(..).max_in_flight(..))`.
- Fail-fast shedding keeps admitted-request latency bounded under overload, and a
  `503` + `Retry-After` is the load balancer's cue to retry elsewhere or back off.
- The mechanism is in the framework; the **policy is the composition root's**. A
  service that sets no controls is unaffected (no layer, no cost), exactly like a
  single-instance deployment that never opts into the distributed rate limit.
- It does **not** replace an edge gateway: a deployment that needs global load
  shedding, circuit breaking or priority queues still puts a dedicated edge in
  front. This is the in-process floor, not the ceiling.

## Alternatives considered

- **A tower `ConcurrencyLimitLayer` + `LoadShedLayer` + `TimeoutLayer` stack.**
  Idiomatic tower, but it introduces `tower::Service` errors that axum requires a
  `HandleError` layer to map back to responses, and it is awkward to key per route
  (the layer wraps a whole service). The `from_fn` middleware with a path-matched
  rule table is simpler, matches the existing rate-limiter, and produces RFC 9457
  problems directly.
- **A single global timeout/concurrency knob.** Rejected: the right budget is a
  property of the route, and one global value is either too tight for slow routes
  or too loose for fast ones.
- **Queue the overflow instead of shedding.** Rejected: an unbounded queue just
  defers the collapse and inflates tail latency; bounded fail-fast shedding is the
  backpressure signal a caller can act on.
