# Operational runbooks

These are the step-by-step procedures for running a Seiva service in production.
Each one is written for an on-call operator: a symptom, how to confirm it, what
to do, and how to know you are done.

They assume the service is deployed with the chart in
[`deploy/seiva-server`](https://github.com/ngolacloud/seiva/tree/main/deploy/seiva-server)
(a non-root distroless pod with `/health/live` and `/health/ready` probes, an HPA
and a PodDisruptionBudget) and that it uses the resilience primitives in
[`seiva-resilience`](https://github.com/ngolacloud/seiva/tree/main/crates/seiva-resilience)
(circuit breaker, retry, timeout).

| Runbook | When to reach for it |
|---------|----------------------|
| [Startup & rollout](startup.md) | Deploying a new version, or a pod will not become ready |
| [Dependency degradation (breaker open)](dependency-degradation.md) | A downstream dependency is failing; the circuit breaker has tripped |
| [Rollback](rollback.md) | A release is bad and must be reverted now |

## The health model, in one paragraph

`/health/live` returns `200` in **every** lifecycle state, including while
draining — a live-but-draining pod must be taken out of rotation, not killed
mid-shutdown. `/health/ready` returns `200` only while the process is `Ready`
**and** its critical-dependency check passes; `Starting`, `Draining`, or a failing
dependency return `503` so the load balancer stops routing to it. A transient
dependency blip therefore sheds traffic (readiness `503`) without triggering a
restart storm (liveness stays `200`). This split is the backbone of all three
runbooks below.

---

*Part of the [Seiva documentation](../index.md).*
