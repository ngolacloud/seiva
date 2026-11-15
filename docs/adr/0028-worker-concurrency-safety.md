# ADR 0028 — Worker concurrency safety: competitive claim with `SKIP LOCKED` and an atomic claim-effect-mark transaction, so scaling a worker to N replicas does not duplicate effects

- Status: Proposed
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0011 (durable idempotency), ADR 0021 (in-process event bus and choreography), ADR 0015 (distributed rate limit)

## Context

Seiva's HTTP path scales horizontally without ceremony: it is stateless and the
rate limit is distributable (ADR 0015). The **workers are not** safe to scale the
same way, and nothing in the scaffold warns about it.

- The outbox reader drains a batch with
  `SELECT ... WHERE topic = $1 AND id > $2 ORDER BY id LIMIT $3` — **no
  `FOR UPDATE SKIP LOCKED`, no lease** (`crates/seiva-outbox-relay/src/lib.rs`).
  `advance` only guarantees the cursor moves forward (`... AND position < $3`).
  Monotonicity stops the cursor from going *backwards*; it does **not** stop two
  replicas reading and processing the **same** batch. Run `(consumer, topic)` on
  two pods and both forward the same events.
- The broker-publish path (`forward`) has **no dedup on the publish side**: N
  replicas publish each event N times. It is at-least-once (consumers dedup via the
  inbox), but the amplification is real waste and load.
- The in-process dispatcher does claim → handle → finish **without a transaction**
  (`crates/seiva-events/src/lib.rs`): a crash between `handle` and `finish` can lose
  the mark, so the effect ran but the event looks unprocessed. The atomic
  claim+effect+mark ("Fase 2") is **documented but not implemented** in-tree.
- The crate comment itself concedes leader election is out of scope: *"The
  surrounding loop (interval, graceful shutdown, metrics, leader election) is the
  caller's worker runtime"* — so the framework ships no primitive for it.

The forces:

- The safe-today workaround (one HTTP Deployment with N replicas + a **separate**
  worker Deployment pinned to `replicas: 1`) is an operational footgun: the obvious
  `replicas: 3` on a Deployment that bundles the worker silently duplicates effects
  and DB load. The framework must make **scaling the worker out the safe default**,
  not a trap.
- The guarantee must be **at-least-once with dedup**, never at-most-once: losing an
  event is worse than handling it twice when every effectful subscriber already
  deduplicates through the durable inbox (ADR 0011). So the fix is about **not
  duplicating work** and **not losing the mark**, not about exactly-once delivery
  (which is a fiction over a broker).
- It must stay within the ports the application already codes against
  (`UnitOfWork`, `IdempotencyStore`, `EventPublisher`) and the DORM gateway — no new
  infrastructure dependency (no ZooKeeper/etcd for leader election).
- Postgres already offers the right primitive (`FOR UPDATE SKIP LOCKED`), so
  competitive consumers can share a queue **without** a single-leader bottleneck —
  preferable to leader election, which caps throughput at one worker.

## Decision

Make the durable outbox a **competitively-claimed queue** and make in-process
handling **atomic**, so workers are safe at N replicas by construction.

**1. Competitive claim with `SKIP LOCKED` (replaces the monotonic cursor for the
multi-reader case).** The `DormOutboxReader` claims a batch inside a transaction:

```
SELECT ... FROM <outbox>
 WHERE topic = $1 AND status = 'pending'
 ORDER BY id
 LIMIT $3
 FOR UPDATE SKIP LOCKED
```

then marks the claimed rows `status = 'processing'` with an `owner` and a
`claimed_at` lease expiry, and commits. Two replicas running the same
`(consumer, topic)` draw **disjoint** batches — work is partitioned, not
duplicated, with no leader and no single-reader bottleneck. A row whose lease
expires (owner crashed before completing) returns to `pending` and is re-claimed —
at-least-once, never stuck. The forward-only cursor remains as a read-side
optimisation only where a single reader is guaranteed.

**2. Atomic claim-effect-mark (the "Fase 2" the event bus documents).** The
in-process dispatcher runs the subscriber inside **one** `UnitOfWork::transaction`:
claim the inbox entry (dedup, ADR 0011) → run the subscriber's effect → mark
processed — all commit together. A crash anywhere rolls the whole thing back, so
the event stays claimable and is re-handled; the effect and its mark can never
diverge. This is the in-process twin of the outbox's transactional guarantee, so a
subscriber behaves identically in a monolith and after `module split`.

**3. Publish-side idempotency on the broker path.** `forward` stamps each event
with a stable idempotency key (envelope id) so a broker/consumer can collapse the
N-replica amplification; combined with (1) the amplification is largely removed at
the source because only the claiming replica forwards a given row.

**4. Scaffold honesty until this ships.** Until (1)–(3) land, the generated project
carries an explicit note and a worker Deployment pinned to `replicas: 1`, so the
unsafe `replicas: N` is a deliberate choice, not an accident.

## Consequences

- Scaling the worker out becomes safe and useful: competitive consumers share the
  outbox via `SKIP LOCKED`, throughput grows with replicas, and a crashed worker's
  in-flight batch is reclaimed after its lease — no leader, no single point.
- The effect/mark split that could drop a mark is closed: claim-effect-mark is one
  transaction, so the system is at-least-once with durable dedup end to end
  (outbox *and* in-process), matching ADR 0011.
- Broker amplification drops from N× to ~1× at the source, with idempotency keys as
  the consumer-side backstop.
- The guarantee is explicitly at-least-once, not exactly-once: effectful
  subscribers must stay idempotent through the inbox — the framework enforces the
  dedup primitive, the subscriber must use it (unchanged from ADR 0011/0021).
- The outbox schema gains `status`, `owner`, `claimed_at` columns; this is a DORM
  migration, additive, applied through `seiva db migrate`. The read path for a
  single-reader deployment is unchanged.
- It needs no external coordinator (no ZooKeeper/etcd): the coordination primitive
  is Postgres row locking, which the DORM gateway already speaks.

## Alternatives considered

- **Leader election (one active worker, others standby).** Rejected as the default:
  it caps throughput at a single worker and needs a lease/coordinator the framework
  deliberately does not ship. `SKIP LOCKED` gives safe concurrency *and*
  horizontal throughput with no coordinator. Leader election remains a valid
  deployment choice for a strictly-ordered consumer, layered by the operator.
- **Keep the monotonic cursor, pin the worker to `replicas: 1` forever.** Rejected:
  it makes the worker a scaling and availability bottleneck and leaves the footgun
  in place. Acceptable only as the interim scaffold note (decision item 4).
- **Kafka-style static partition assignment.** Rejected for the in-tree durable
  path: it presumes a partitioned log and a rebalance protocol Seiva does not have.
  `SKIP LOCKED` over the SQL outbox is the simplest thing that is correct here; a
  Kafka backend can still be a messaging adapter behind the `Publisher` port.
- **At-most-once (mark before effect).** Rejected outright: it trades duplicate
  work (cheap, deduplicated) for lost effects (silent data loss), the wrong
  direction for a high-criticality framework.
