# ADR 0012 — Cloud-native health probes and rate-limit posture

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0001

## Context

The brief (§8) sets operational rules a backend must honour under an orchestrator
and a load balancer:

- **Readiness** reflects initialization and *critical dependencies*, with checks
  that are **limited/cached** so a probe flood does not hammer the dependency.
- **Liveness** must not trigger restart storms: a transient dependency failure
  must not make the orchestrator kill a healthy process.
- **Health probes must not share the clients' rate-limit budget** — a load
  balancer polls `/health/*` constantly, and many clients can sit behind one
  source IP (a NAT, the proxy itself).

The framework already had `/health/live` (always `200`) and `/health/ready`
(`200` only while `Ready`), but both were *inside* the rate limiter in
`apply_framework_layers`, and readiness could not reflect a dependency.

## Decision

- **Health probes are exempt from the rate limiter.** The `rate_limit`
  middleware skips `/health/live` and `/health/ready` before keying on the
  client, so probe traffic (or many clients behind one IP) can never trip a
  `429` and get the instance marked unhealthy.
- **Readiness can carry a cached critical-dependency check.**
  `health_router_with_dependency(health, check, ttl)` takes a `ReadinessCheck`
  (a closure returning a boxed future). `/health/ready` is `200` only while
  `Ready` **and** the check passes; the result is cached for `ttl`, so the check
  runs at most once per window however often the probe is polled. The default
  `health_router` keeps the lifecycle-only behaviour.
- **Liveness never consults a dependency.** `/health/live` keys off the lifecycle
  state alone, so a transient database blip makes readiness `503` (taken out of
  rotation) while liveness stays `200` (not restarted) — no restart storm.
- **The rate limiter is per-process, and says so.** It is a fixed-window,
  in-memory limiter keyed by client IP (the trusted rightmost `X-Forwarded-For`
  entry only when a single trusted edge is configured, never a blind boolean).
  Across replicas each process enforces its own budget; a globally-consistent
  limit needs a shared backend, documented as a later integration.

## Consequences

**Positive.** Probes survive floods and NAT'd clients; readiness tracks a real
dependency without hammering it; a dependency blip never restarts a healthy
process; the proxy-trust rule resists `X-Forwarded-For` spoofing.

**Negative / cost.** The per-process limiter is not a global quota across
replicas (acceptable for abuse-dampening; a distributed backend is future work).
The readiness cache means a dependency recovery is visible only after at most
`ttl`.

## Alternatives considered

- **Mount health outside every layer.** Rejected: health should still carry the
  security headers and the correlation id; only the *rate limit* (and the body
  limit, irrelevant to a GET) must be skipped — a targeted path exemption is
  simpler than splitting the whole layer stack.
- **Readiness always probes the dependency (no cache).** Rejected: a probe flood
  would become a dependency flood (§8).
- **Liveness probes the dependency too.** Rejected outright: it is the classic
  restart-storm cause the brief names.
