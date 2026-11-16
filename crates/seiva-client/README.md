# seiva-client

The inter-service client for [Delonix Seiva](https://github.com/ngolacloud/seiva):
how one module calls **another module's service** once they are split into
separate microservices, with the framework's resilience guardrails wrapped around
every call.

It is the first real consumer of [`seiva-resilience`](../seiva-resilience). A
call runs inside a **circuit breaker**, retries a transient failure on the
framework's one **retry schedule** (`seiva_resilience::RetryPolicy`), and **times
each attempt out** — composed from the inside out:

```text
breaker.call(                     // shed load if the dependency is already down
    retry.retry(                  // absorb a transient failure (one schedule)
        timeout(budget, attempt)  // cut a slow attempt off
    )
)
```

The breaker records **one** outcome per `call`, not one per retried attempt: a
transient failure the retry absorbs is invisible to the breaker, so a flaky
dependency does not trip it; a dependency that is genuinely down (every attempt
fails) counts as a single failure and, after enough of them, trips it open.

## Transport-agnostic

The crate names no HTTP or gRPC client. It composes futures around a
[`Transport`] seam the caller injects — the same adapter pattern as the rest of
the framework — so it builds and tests fully **offline** against an in-memory
transport. The concrete HTTP/gRPC transport is a downstream adapter, not a
dependency of this crate.

Data-layer-free (DORM-free).
