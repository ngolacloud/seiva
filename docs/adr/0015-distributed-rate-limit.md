# ADR 0015 — Distributed rate limiting as an optional port

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0012 (cloud-native health & limits)

## Context

The framework's rate limiter (ADR 0012) is **in-process**: each instance keeps
its own fixed-window counters. That is correct for a single instance, but behind
a load balancer with N replicas a client can get up to N × the limit by spreading
requests across instances. The brief (§8) asks to document the per-process
limitation and **offer a distributed integration when needed** — without making
the base depend on a specific backend, and without pretending a cache is a
durable business guarantee (rate limiting is abuse-dampening, not business state).

## Decision

**A `RateLimitStore` port expresses a shared, atomic rate-limit backend; a
`rate_limit_with_store` middleware enforces it; an optional
`seiva-ratelimit-redis` crate implements it on Redis. The base keeps its
in-process limiter as the default.**

- `seiva_ports::RateLimitStore::check(key, limit, window) -> RateLimitDecision`
  (`Permitted` | `Limited { retry_after }`). The contract requires the backend to
  **increment-and-expire atomically** (one round trip), so concurrent replicas
  cannot race past the limit.
- `seiva_http::rate_limit_with_store` reuses the exact client-key extraction
  (peer IP, or the rightmost `X-Forwarded-For` only behind a single trusted edge)
  and health-probe exemption of the in-process limiter, and returns the same
  `429` + `Retry-After`. If the store errors (a transient outage) the request is
  **permitted** — a limiter outage must not drop legitimate traffic.
- `seiva-ratelimit-redis::RedisRateLimitStore` runs a single atomic Lua script
  (`INCR`; `PEXPIRE` on the first hit of the window; read `PTTL`). It is
  **optional** and not a base dependency; a multi-replica deployment opts in.
- `seiva_testing::InMemoryRateLimitStore` doubles the port for logic tests.

### Proof

Unit: `rate_limit_with_store` permits up to the limit then `429`s, and exempts
health, driven by the in-memory store. **Live** (`seiva-ratelimit-redis`, gated on
`SEIVA_REDIS_TEST_URL`) against a real Redis: 50 concurrent clients share one
budget and exactly `limit` are permitted (the rest `Limited` with a positive
retry-after) — the cross-replica guarantee the in-process limiter cannot give.
Ran green against Redis 7.

## Consequences

**Positive.** A real distributed option, atomic across replicas, with the same
key/exemption/response policy as the in-process limiter; the base stays
backend-agnostic behind the port; a store outage fails open.

**Negative / cost.** A Redis round trip per rate-limited request (a dedicated
edge proxy may still be preferable at very high RPS — unchanged guidance). The
Redis fixed window is per-client-per-window, like the in-process one (not a
smoother token bucket).

## Alternatives considered

- **Rate limiting in PostgreSQL.** Rejected: a DB round trip per request couples
  rate limiting to the business database and is a worse fit than Redis for
  high-frequency counters; §8 also warns against overloading the SQL authority.
- **Replace the in-process limiter.** Rejected: the in-process one is the right
  default for a single instance and is well-tested; the distributed store is
  additive and opt-in.
- **Make Redis the authority for idempotency too.** Rejected (ADR 0011): durable
  business idempotency needs atomicity with the PostgreSQL mutation + outbox,
  which Redis cannot provide; rate limiting has no such requirement.
