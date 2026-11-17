# Runbook: dependency degradation (circuit breaker open)

**Use when:** a downstream dependency (a database, a broker, another service's
API) is slow or failing, and the service's **circuit breaker has tripped open**.

## What an open breaker means

A [`CircuitBreaker`](https://github.com/ngolacloud/seiva/tree/main/crates/seiva-resilience)
wraps a dependency call. After `failure_threshold` consecutive failures it moves
to **Open** and **rejects calls without running them** (`BreakerError::Open`),
shedding load off the dependency so it is not hammered while already down. After
`open_cooldown` it admits a bounded number of probe calls (**HalfOpen**); enough
successes close it, any failure re-opens it and restarts the cooldown.

```text
Closed ──(failure_threshold failures)──▶ Open ──(cooldown)──▶ HalfOpen ──(successes)──▶ Closed
                                          ▲                        │
                                          └────(a probe fails)─────┘
```

This is **working as designed**: the breaker is protecting the service and the
dependency. Your job is to find *why the dependency is failing* and restore it —
not to disable the breaker.

## Confirm it

Each transition is logged (`tracing`) by the breaker:

```console
$ kubectl logs -l app.kubernetes.io/name=seiva-server --tail=200 \
    | grep -i "circuit breaker"
# "circuit breaker tripped; shedding load off the dependency"  state=open
# "circuit breaker probing recovery"                           state=half_open
# "circuit breaker recovered"                                  state=closed
```

If the breaker guards a **critical** dependency, the readiness probe reflects it
and the pod is pulled from rotation:

```console
$ curl -s localhost:8080/health/ready
{"status":"unready","state":"dependency-unready"}   # HTTP 503
```

Liveness stays `200` throughout, so Kubernetes does **not** restart the pod — a
restart would not fix a downstream outage and would only drop in-flight work.

## Diagnose the dependency

The breaker is the symptom; the dependency is the cause. Work outward:

1. **Which dependency?** The log line and the breaker's metric label name it.
2. **Is it reachable from the pod?**
   ```console
   $ kubectl exec <pod> -- true   # distroless has no shell; use a debug container:
   $ kubectl debug <pod> -it --image=nicolaka/netshoot -- \
       sh -c 'curl -v http://<dependency>/health; nslookup <dependency>'
   ```
3. **Is the dependency itself healthy?** Check its own dashboards/logs (DB
   connections maxed, broker down, a partner API returning `5xx`).
4. **Is it a timeout, not an outage?** If calls are slow rather than failing, the
   `timeout` wrapper is converting slowness into failures that trip the breaker.
   The dependency is overloaded — look at its latency, not just its error rate.

## Act

- **If the dependency is the problem** (the usual case): fix it there — scale it,
  restart it, fail over, or escalate to its owner. **Do nothing to the breaker.**
  Once the dependency recovers, the breaker's next probe succeeds and it closes
  automatically (watch for the `state=closed` log line). No deploy is needed.
- **If the breaker is mistuned** (tripping on normal transient noise, or taking
  too long to recover), adjust `BreakerConfig` at the composition root and roll a
  new image — `failure_threshold`, `open_cooldown`, `success_threshold`. This is a
  **code/config change**, not an incident action; do it after the incident.
- **If a non-critical dependency is down** and you want the service to keep
  serving degraded (the breaker short-circuiting to a fallback), confirm the call
  site has a fallback path; an open breaker with no fallback surfaces as an error
  to the caller, which may be the correct behaviour.

!!! warning "Do not 'fix' it by removing the breaker"
    Disabling the breaker during an incident removes the only thing stopping the
    service from hammering a dependency that is already on its knees — it turns a
    contained degradation into a cascading failure. Fix the dependency.

## Done when

- The breaker log shows `state=closed` (recovered), or the dependency owner
  confirms the dependency is healthy again.
- `/health/ready` returns `200` and the pods are back in rotation.
- Error rate and latency to the dependency are back to baseline on the dashboard.

## Related

- [Startup & rollout](startup.md) — the readiness/liveness split this relies on
- [Rollback](rollback.md) — if the degradation was caused by a bad release, not a dependency

---

*Part of the [Seiva documentation](../index.md).*
