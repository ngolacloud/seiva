# Production-path load benchmark

This is the companion the [build-profile note](llvm-profile.md) defers to: it keeps
the **secure path on** — token authentication, a tenant-scoped read/write through
**DORM over a real PostgreSQL**, response security headers and the per-IP rate
limiter — and reports latency percentiles, throughput and status codes. A smaller
binary is a build property; this measures the running service doing real work.

The harness is [`benchmarks/production/run.sh`](../../../benchmarks/production/run.sh).
It builds the `admin-data-e2e` service (the `Catalog` CRUD module from ADR 0009's
addendum: five operations over a `catalog_items` table via `DormUnitOfWork`), runs
**two replicas as two processes sharing one PostgreSQL** (the shared-state replica
model), and drives [`oha`](https://github.com/hatoo/oha) against them.

## What was measured

- Host: single 20-core x86-64, loopback networking, PostgreSQL 16 over the Unix
  socket (peer auth). `oha 1.16.0`, 50 connections, 10 s per case.
- Service: release profile (ThinLTO, `codegen-units = 1`, strip). DORM pool
  `pool_max = 8` per replica. Authentication **on** (`clerk-token` → a `Principal`
  in tenant `acme`). Every read is tenant-scoped (`WHERE tenant = $1`) and masks the
  `cost` column by permission; every write is a real `INSERT ... RETURNING`.

Two scenarios, both run as-is — nothing suppressed:

### Scenario 1 — Protected (the secure default: 300 req/min per IP)

A 50-connection flood against one replica with the shipped rate limit
(`RateLimit::default` = 300 requests / 60 s, keyed by client IP). All load comes
from one IP, so after the 300-request budget the limiter sheds the rest:

| Case | req/s | p50 | p95 | p99 | 2xx | 429 |
|---|---:|---:|---:|---:|---:|---:|
| `GET /v1/catalogs` | 255 955 | 0.16 ms | 0.42 ms | 0.74 ms | 298 | 2 563 826 |

The point is **not** the throughput number — it is that the DoS protection rejects
a flood in sub-millisecond time (p99 0.74 ms) after admitting exactly its budget.
The limiter is the work here; the database is barely touched.

### Scenario 2 — Work path (rate limit raised so it does not bind)

The same path with the limiter raised out of the way
(`ADMIN_DATA_E2E_RATE_LIMIT`), so the measurement is the **cost of auth + the
tenant-scoped DB read/write**, not the limiter:

| Case | Replica | req/s | p50 | p95 | p99 | 2xx |
|---|---|---:|---:|---:|---:|---:|
| `GET /v1/catalogs` (auth + scoped SELECT) | r1 | 48 156 | 1.00 ms | 1.35 ms | 1.74 ms | 481 639 |
| `GET /v1/catalogs` (auth + scoped SELECT) | r2 | 46 511 | 1.00 ms | 1.52 ms | 2.40 ms | 465 310 |
| `POST /v1/catalogs` (auth + INSERT) | r1 | 10 630 | 3.36 ms | 8.24 ms | 32.10 ms | 106 274 |

- **Read, aggregated across the two replicas: ≈ 94.7 k req/s** of authenticated,
  tenant-scoped PostgreSQL-backed reads, p99 under 2.5 ms.
- **Write: ≈ 10.6 k req/s** of authenticated inserts that persist to PostgreSQL;
  the write p99 (32 ms) is dominated by PostgreSQL commit + the 8-connection pool
  under 50-way contention, exactly where it should show.
- A handful of `errors` reported per case (≈ 50 = one per connection) are the
  connection resets at shutdown, negligible against hundreds of thousands of
  requests.

### Memory

One replica, measured from `/proc/<pid>/status` `VmRSS`:

| State | RSS |
|---|---:|
| Idle (ready, no traffic) | 6 MiB |
| Peak under 50-connection load | 15 MiB |

## Honest caveats — what this is and is not

- **Single host, loopback.** No network, no real client distribution, no TLS
  termination hop. Numbers are indicative of the *path's cost on this box*, not a
  capacity SLA. Re-measure on your target.
- **Two processes model two replicas.** They genuinely share one PostgreSQL (the
  real contention point), but there is no load balancer and no cross-host latency.
- **In-process rate limit here, not the distributed one.** This binary uses the
  per-process fixed-window limiter. The distributed Redis limiter
  (`seiva-ratelimit-redis`, ADR 0014) is what makes the per-IP budget hold *across*
  replicas; it is proved by its own live test, not exercised in this run.
- **Idempotency not on the hot path of this service.** The durable idempotency
  store (ADR 0012) is proved by `seiva-data-dorm`'s multi-process live test; this
  `Catalog` write path does not attach an `Idempotency-Key`, so these write numbers
  are the raw INSERT cost, not the de-duplicated-write cost.
- **This is not the overhead microbenchmark.** The build-profile note deliberately
  turns auth off to isolate framework overhead; this one keeps the whole secure
  path on. They answer different questions — do not compare their throughput numbers
  directly.

## Reproduce

```sh
createdb seiva_prod_bench   # once
PROD_BENCH_DURATION=10s PROD_BENCH_CONNECTIONS=50 bash benchmarks/production/run.sh
# raw oha JSON lands in benchmarks/production/results/
```

A fully containerised, orchestrated multi-replica deployment behind a load
balancer (via `delonix`), with the distributed rate-limit and idempotency on the
hot path, is the production form and a separate, larger exercise — tracked as an
open gap, not claimed here.
