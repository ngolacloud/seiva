# ADR 0033 — A durable, retrying webhook delivery engine on top of the existing `seiva-webhooks` signing

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0011 (durable idempotency), ADR 0021 (in-process event bus and choreography), ADR 0026 (per-route load control), ADR 0034 (durable job queue)

## Context

Seiva already signs webhooks correctly. `seiva-webhooks` implements the
[Standard Webhooks] scheme — an `HMAC-SHA256` over `{id}.{timestamp}.{payload}`,
the `webhook-id` / `webhook-timestamp` / `webhook-signature` headers, a
trusted-clock replay window, multi-signature key rotation. That is the hard
*cryptographic* half, and it is done. **The signing is not the gap.**

What is missing is everything around the signature: taking a domain event,
turning it into an outbound HTTP call to a tenant's endpoint, and then making that
call **reliably** — because a receiver is down, slow, or flaky far more often than
it is healthy-and-instant. Today a product that wants "fire a webhook when X
happens" has to hand-roll the delivery loop: who to send to, retry on failure, how
many times, with what backoff, where failures go when the endpoint is dead for an
hour, and how to stop a wall of retries against one broken endpoint from drowning
the others. delonix-meet does exactly this by hand. That loop is a horizontal
*mechanism* — a durable, retrying delivery engine — and Seiva already owns every
primitive it needs (`seiva-webhooks` to sign, `seiva-resilience` to back off,
`seiva-jobs` to make it durable, the event bus to feed it). It just has not been
assembled.

The forces:

- **Extend, do not reinvent.** The HMAC signing stays exactly as `seiva-webhooks`
  ships it. This ADR adds the *delivery* around it; it changes no byte of the
  signature.
- **Delivery is durable, off the request path.** An event must be recorded as owed,
  then delivered asynchronously — a slow receiver must never hold an API request
  open (the opposite of ADR 0026's shedding). That durable work sits on the job
  queue (ADR 0034), so a crash never loses a pending delivery.
- **At-least-once with idempotency.** A retried delivery may arrive twice; each
  carries a stable idempotency key (the message id) so the receiver deduplicates
  (ADR 0011). The engine never pretends exactly-once over the network.
- **Per-endpoint isolation.** One tenant's dead endpoint must not starve delivery
  to everyone else. Retries need bounded backoff + jitter, a dead-letter after N
  attempts, and a **per-endpoint circuit breaker** so a persistently-failing target
  stops being hammered.
- **Egress-safe.** A tenant-supplied URL is an SSRF vector; the outbound address is
  validated against private/metadata ranges before the call (the guard formalised
  in ADR 0045 — until it lands, a conservative in-engine check).

## Decision

Add a **delivery engine** crate, `seiva-webhook-delivery`, that composes the
existing primitives; `seiva-webhooks` stays the signing library, untouched.

- **A `deliveries` ledger** — one row per `(subscription, event)` delivery, with
  `status` (`pending` → `in_flight` → `delivered` | `failed` | `dead`), the
  `attempt` count, `next_attempt_at`, the last HTTP status and a response excerpt.
  The ledger is the durable source of truth for what is owed and what happened.
- **Fed by the event bus.** A `seiva-events` subscriber (or an outbox drain, ADR
  0021) turns a published domain event into one `pending` row per matching
  subscription, **inside the same transaction** as the ledger write — so a committed
  event always has its deliveries enqueued, exactly like the idempotency reservation
  (ADR 0011).
- **Driven by the job queue.** Delivery is a job on `seiva-jobs` (ADR 0034): a
  worker claims due `pending` rows under a lease, signs the payload with
  `seiva-webhooks`, validates egress, POSTs, and records the outcome. Running on the
  queue means the whole thing is durable, leased and safe at N replicas by
  construction.
- **Retry on `seiva-resilience`.** A transient failure (timeout, 5xx, connection
  error) schedules the next attempt with **exponential backoff + jitter**
  (`RetryPolicy`), bounded `timeout()` per POST, up to a max attempt count; past the
  cap the row goes **dead-letter** (`dead`) for inspection/replay rather than
  retrying forever.
- **Per-endpoint circuit breaker.** A `CircuitBreaker` keyed by destination endpoint
  opens after sustained failures, so a dead endpoint is skipped (its rows deferred)
  instead of consuming workers — isolating one broken receiver from the rest.
- **Idempotency key per delivery.** The `webhook-id` doubles as the receiver's
  idempotency key; a redelivered attempt carries the same id so the receiver
  deduplicates (ADR 0011). Delivery is explicitly at-least-once.
- **Egress validation.** Before every POST the destination is checked against an
  allow/deny policy (block loopback, link-local, RFC 1918, cloud-metadata
  `169.254.169.254`) — the SSRF guard that ADR 0045 will make a shared primitive.

## Alternatives considered

- **A new signing scheme bundled with delivery.** Rejected outright: `seiva-webhooks`
  already implements Standard Webhooks correctly, reviewed. The engine composes it;
  re-deriving HMAC would fork a solved, security-sensitive primitive.
- **Deliver synchronously inside the request that emits the event.** Rejected: it
  ties API latency to a third-party endpoint's health and loses the delivery on a
  crash. Durable enqueue + async delivery is the only shape that survives a receiver
  being down and a process restart.
- **A bespoke retry loop instead of `seiva-jobs` + `seiva-resilience`.** Rejected: it
  re-derives leases, backoff, dead-letter and multi-replica safety that the framework
  already owns. The engine is assembly of existing primitives, not new machinery.
- **Best-effort fire-and-forget (no ledger).** Rejected: without a durable record of
  what is owed there is no retry, no dead-letter, no audit of what a tenant was (not)
  told — the opposite of a reliable integration surface.

## Consequences

- **Performance.** Delivery runs off the request path on the job queue, so emitting
  an event is a cheap durable write; the worker batches due rows and a slow receiver
  never blocks an API caller. The crate is opt-in — a service that fires no webhooks
  pays nothing.
- **Security.** Payloads are signed by the existing `seiva-webhooks` `HMAC-SHA256`;
  the destination is **egress-validated** against SSRF ranges (ADR 0045) before every
  POST; secrets and signatures are never logged; delivery is over TLS. The ledger
  records outcomes without storing the signing secret.
- **Resilience.** Retry with bounded exponential backoff + jitter, a bounded per-POST
  timeout, **dead-letter** after N attempts, and a **per-endpoint circuit breaker** so
  one dead receiver cannot starve the rest; at-least-once delivery with the `webhook-id`
  as the receiver's idempotency key (ADR 0011); durability and N-replica safety inherited
  from the job queue (ADR 0034) and outbox (ADR 0021).
- delonix-meet keeps its **event catalogue and per-tenant subscriptions** as policy and
  deletes its hand-written delivery loop; "which events fire and to whom" stays in the
  product, "deliver it reliably and prove it" becomes the framework's.
- It does **not** become a general message broker or an inbound webhook receiver: it is
  the outbound-delivery engine for a product's own event catalogue, sitting on the queue
  and the signer the framework already has.

[Standard Webhooks]: https://www.standardwebhooks.com/
