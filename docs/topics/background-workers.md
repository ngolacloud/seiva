# Background work and the worker runtime

Some work happens outside a request: draining the outbox, polling, periodic jobs.
Seiva's worker runtime (`delonix_seiva::worker`) hosts these alongside — or
instead of — the HTTP server.

## The worker runtime

A worker is a loop the runtime drives: it does a unit of work on an interval,
reports metrics (a `WorkerReport`: passes, processed, errors), and stops cleanly on
the shutdown signal. `seiva-server` can host workers and the HTTP app together, or
a worker can be its own `main`.

```rust
run_worker(WorkerConfig { interval, .. }, shutdown_signal(), || async {
    // one pass of work; return how many items were processed
}).await
```

## The headline consumer: the outbox relay

The transactional-outbox relay (`seiva-outbox-relay`) is a worker: each pass drains
the outbox and forwards events to the broker, tracking a cursor and counting
errors, until shutdown. This is what makes event delivery reliable — see
[messaging and events](messaging-and-events.md).

## When to use a worker

- Forwarding the outbox to a broker (the relay).
- Periodic maintenance (expiry, reconciliation, cache warming).
- Polling an external system on an interval.

For reacting to another service's events, prefer a broker consumer (driven by the
messaging layer) over polling; use a worker when the cadence is time-based or the
source has no push.

## Graceful shutdown

Workers stop on the same `SIGTERM` that drains the HTTP server — the runtime
resolves the shutdown future, the loop finishes its current pass, and the process
exits cleanly. A rollout neither drops a request nor abandons a pass mid-flight.

## Related

- [Messaging and events](messaging-and-events.md) · [The application and composition root](transport/application-and-composition.md)

---

*Part of the [Seiva documentation](../index.md).*
