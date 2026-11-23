# ADR 0031 — Native real-time: a first-class WebSocket + SSE hub as a framework primitive, protocol-neutral and multi-pod

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0012 (cloud-native health and limits), ADR 0019 (fail-closed authorization), ADR 0021 (in-process event bus and choreography), ADR 0026 (per-route load control)

## Context

Seiva's HTTP transport is strictly **request/response**. ADR 0026 even bounds a
route by shedding connections (`503`) — the opposite of holding one open. There
is no WebSocket, no SSE, no streaming anywhere in the framework, and the roadmap
does not mention one.

But a framework that aims to be the base for **cloud-native platforms** cannot
not have a push channel. Live collaboration, presence, dashboards that update in
place, notifications, log/event streams, and signalling for anything WebRTC all
need the server to push to the client. The driver here, delonix-meet, builds this
by hand: a 10 000-line `signaling.rs` on `axum::ws`, auth by a short-lived
room-token, in-memory connection state in a `DashMap`, and Redis pub/sub
(`pubsub.rs`, `redis_state.rs`, `presence.rs`) to fan events across pods. That is
a generic hub with a domain protocol bolted on — exactly the shape that should be
a framework primitive, built once, correctly.

The forces:

- **Protocol-neutral.** The framework must carry *frames*, not *meanings*. It must
  know nothing of "offer/answer/ice"; the message schema is the product's policy.
- **A persistent connection is a different resource than a request**, and needs its
  own limits: a per-process connection cap, an idle/heartbeat timeout, a **bounded
  send buffer** so one slow consumer cannot grow memory without bound, and a
  per-tenant cap. ADR 0026's request semaphore does not cover long-lived sockets.
- **Multi-pod by construction.** A client on pod A must receive what pod B
  publishes to the same topic. The hub needs a pub/sub bridge, not just local
  in-memory fan-out, or horizontal scaling silently drops messages.
- **Fail-closed and origin-safe.** The upgrade must authenticate *before* it
  completes (deny-by-default, ADR 0019), bind the socket to a `Principal`+`Scope`,
  and check `Origin` to stop cross-site WebSocket hijacking (CSWSH). A short-lived
  **connection-token** (minted by a normal authenticated REST call) is the right
  credential — not a long-lived session cookie riding the upgrade.
- **Zero-cost when unused and DORM-free.** A service that pushes nothing links no
  runtime and pays nothing, like the rate limiter (ADR 0015) and load control.

## Decision

Add a new workspace member **`seiva-realtime`** — data-layer-free, feature-gated,
protocol-neutral.

- **`Hub`** — the connection registry: `ConnectionId → Connection`, grouped by a
  typed **`Topic`** (an opaque interned string, e.g. a product's `room:{id}`).
  Sharded (`DashMap`) for lock-contention-free subscribe/unsubscribe/fan-out.
  `publish(topic, frame)` delivers to every local subscriber **and** hands the
  frame to the bridge for other pods.
- **`Frame`** — an opaque payload (`Bytes` or JSON value). The product's message
  type implements a thin `RealtimeMessage` trait (encode/decode); the hub never
  parses it. Signalling, chat, presence deltas are all the product's policy.
- **Transports, composed at the root**, both authenticating *before* upgrade:
  - **WebSocket** (`axum` `ws` feature) for bidirectional flows (signalling).
  - **SSE** for server→client-only flows (dashboards, notifications) — cheaper,
    proxy-friendly, auto-reconnecting.
- **`RealtimeAuth`** — a trait that validates the connection-token and returns a
  `Principal`+`Scope` (reuses `seiva-identity`); rejection denies the upgrade. An
  `Origin` allowlist is enforced in the same step.
- **Backpressure per connection** — each socket owns a **bounded** `mpsc` send
  queue; when a slow consumer fills it, the connection is **closed** (slow-consumer
  shedding), never buffered unbounded. A ping/pong heartbeat with an idle timeout
  reaps dead sockets.
- **Limits** — a per-process `max_connections` semaphore (breach → upgrade refused,
  the long-lived analogue of ADR 0026), a per-tenant cap, and a max frame size.
- **Multi-pod bridge** — a `BroadcastBackend` trait with a Redis pub/sub adapter
  (and a NATS adapter reusing `seiva-messaging`), plus a `PresenceBackend` for
  aggregated presence. Local-only is the default (no bridge, no cost).
- **Bridge from the event bus** — a `seiva-events` subscriber (or an outbox
  drain) can fan a domain event out to a `Topic`, so "something happened in the
  domain" becomes "connected clients are told" without the product rewiring
  transports. This is the internal-event → client-push seam, made first-class.

`tokio` enters with `default-features = false` (`sync`, `time`, `macros`); no
runtime is linked into the library. The Redis/NATS bridges are optional features.

## Alternatives considered

- **Add WebSocket inside `seiva-http`.** Rejected: a long-lived connection has a
  different resource and failure model than a request (send backpressure, idle
  reaping, per-connection caps). A separate crate keeps the request path's
  reasoning (ADR 0026) clean and lets realtime evolve its own limits.
- **SSE only.** Rejected: signalling is bidirectional; SSE is server→client only.
  We ship **both** — SSE for one-way push, WS for two-way — so the product picks
  the cheapest transport that fits.
- **Leave every product to build on `axum::ws` directly (the status quo).**
  Rejected: that re-derives the hub, pub/sub, presence, backpressure and — most
  dangerously — the *security* (token, origin, caps) in every product, each
  slightly differently. The failure modes here are subtle; building them once,
  reviewed once, is the point of a framework.
- **Carry typed domain messages in the framework.** Rejected: it would weld the
  hub to a protocol. Opaque frames + a product-owned `RealtimeMessage` keeps the
  mechanism-vs-policy line (the whole upstreaming principle).

## Consequences

- **Performance.** Sharded registry for contention-free fan-out; a bounded send
  queue caps per-connection memory; SSE offered as the cheaper one-way path. The
  crate links no runtime and is zero-cost when a service opts out.
- **Security.** Deny-by-default upgrade (authenticate before completing), socket
  bound to `Principal`+`Scope`, `Origin` allowlist against CSWSH, short-lived
  connection-tokens instead of long-lived cookies, frame-size bound. Fail-closed,
  consistent with ADR 0019.
- **Resilience.** Heartbeat + idle reaping drop dead sockets; slow-consumer
  shedding protects memory under a stalled client; the multi-pod bridge makes
  horizontal scaling correct instead of silently lossy; per-process and per-tenant
  caps bound resource use under load.
- Seiva stops being "a framework for APIs" and becomes "a framework for
  platforms": this is the capability whose absence blocked the delonix-meet
  rewrite and that every real-time platform needs.
- delonix-meet reuses the hub and mounts its signalling protocol as policy on top,
  deleting its pub/sub, presence and connection-plumbing boilerplate; the WebRTC
  media plane stays out of the framework (it is not transport, it is media).
- It does **not** replace a dedicated edge for extreme fan-out (millions of
  subscribers): this is the in-process real-time floor, with a pub/sub bridge for
  normal horizontal scale — not a global message bus.
