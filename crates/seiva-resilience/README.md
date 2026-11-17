# seiva-resilience

Production resilience primitives for [Delonix Seiva](https://github.com/ngolacloud/seiva):
the guardrails a service puts between itself and a dependency that can fail, time
out or fall over. **Data-layer-free** (no DORM, no database, no broker) — it
composes futures and times transitions off a monotonic clock.

## What's in it

| Primitive | What it does |
|-----------|--------------|
| [`CircuitBreaker`] | Stops hammering a dependency that is already down: `Closed → Open → HalfOpen → Closed`. An open breaker rejects without running the operation, shedding load; after a cooldown it admits a bounded number of probes to detect recovery. |
| [`RetryPolicy`] | Retries a transient failure with exponential backoff — the **same schedule** (`base` doubling, capped at 30× base) as the transactional-outbox relay (`seiva_events::RelayPolicy` / `dorm_outbox::relay`), so the framework has one retry cadence. |
| [`timeout`] | Cuts an operation off with a typed `TimedOut` error if it outlives its budget, so a slow dependency cannot pin a handler forever. |

## The circuit breaker, in one picture

```text
             failure_threshold consecutive failures
  ┌────────┐ ───────────────────────────────────▶ ┌──────┐
  │ Closed │                                       │ Open │
  └────────┘ ◀─────────────────────────────────── └──────┘
      ▲         success_threshold probe successes      │
      │                                                │ open_cooldown elapsed
      │              any probe fails                   ▼ (one probe admitted)
      └──────────────────────────────────────── ┌──────────┐
                                                 │ HalfOpen │
                                                 └──────────┘
```

## Usage

```rust,no_run
use std::time::Duration;
use seiva_resilience::{CircuitBreaker, BreakerConfig, RetryPolicy, timeout};

# async fn example() {
let breaker = CircuitBreaker::new(BreakerConfig {
    failure_threshold: 5,
    open_cooldown: Duration::from_secs(30),
    half_open_max: 1,
    success_threshold: 1,
});
let retry = RetryPolicy { max_attempts: 3, backoff_base_ms: 50 };

// timeout each attempt, retry transient failures, guard the whole with the breaker
let result = breaker
    .call(async {
        retry
            .retry(|| async {
                timeout(Duration::from_secs(2), call_dependency()).await
            })
            .await
    })
    .await;
# }
# async fn call_dependency() -> Result<(), std::io::Error> { Ok(()) }
```

See the operational runbooks in [`docs/runbooks/`](../../docs/runbooks/) for how an
open breaker shows up in production and how to respond.

[`CircuitBreaker`]: https://docs.rs/seiva-resilience
[`RetryPolicy`]: https://docs.rs/seiva-resilience
[`timeout`]: https://docs.rs/seiva-resilience

## License

MIT OR Apache-2.0, like the rest of the Seiva workspace.
