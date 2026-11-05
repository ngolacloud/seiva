# Performance and cloud-native readiness

Seiva's structural advantage over FastAPI and Django is that it is **native and
compiled**: no interpreter, no GC, one binary. This page is where that cashes out.

## No GC, no runtime

A Seiva service is a Rust binary on an async runtime (tokio). There is no
garbage-collector pausing under load and no interpreter overhead per request, so
throughput is high and — more importantly for a critical platform — **tail
latency is predictable** (no GC stop-the-world at p99). This is the "native, not
interpreted" bet: the win is not just average throughput, it is the shape of the
latency distribution.

## One distroless, non-root binary

The generated `Delonixfile` builds a multi-stage, **distroless** image: only the
binary and its runtime libraries, no shell, no package manager, running as uid
65532. Deploy is "copy one binary", not "ship a Python/Node runtime and its
dependency tree". Smaller image, smaller attack surface, faster cold start. See
[tutorial part 6](../getting-started/tutorial/part-6-packaging-and-deploy.md).

## Readiness and graceful drain

`/health/ready` and `/health/live` expose a `Starting → Ready → Draining`
lifecycle; `SIGTERM` drains in-flight requests before exit. A rolling update neither
drops a request nor serves one before the service is ready.

## Efficient by construction

- Validated bodies are bounded (a default body-size limit) before a handler runs.
- The data layer uses SQL joins, not per-row fetches (no N+1 — see
  [queries](data-layer/queries.md)), and keyset pagination for lists.
- CQRS query-cache middleware caches reads where invalidation is known
  ([caching](caching.md)).

## Benchmarking

The honest way to back the performance claim is a published, reproducible
benchmark against FastAPI/Django on the same workload and hardware (throughput and
p50/p95/p99). The architectural reasons above are *why* the numbers should favour
Seiva, but the numbers themselves should be measured and published, not asserted.

### Measured

That harness now exists in [`benchmarks/`](https://github.com/ngolacloud/seiva/tree/main/benchmarks)
(`cd benchmarks && ./run.sh`): a minimal Seiva service against equivalent FastAPI
and Django services on the same trivial, **no-database** workload, so it isolates
framework overhead. On the reference machine (20-core i9-13900H, loopback, `oha`
at 50 connections, 20 s, 5 s warmup; Python servers at 20 workers) Seiva served
roughly **3–5× the throughput of FastAPI and ~15× that of Django**, with p99 under
**0.8 ms** on both `GET /health` and a validating `POST`. Full table, versions and
caveats: [Benchmarks](../benchmarks.md). The figures are environment-dependent —
re-run the harness to measure your own hardware.

## Related

- [Configuration](configuration.md) · [Observability](observability.md) · [Competitive landscape](../competitive-landscape.md)

---

*Part of the [Seiva documentation](../index.md).*
