# Benchmarks: Seiva vs FastAPI vs Django

This page backs the "native, not interpreted" claim with **measured** numbers,
not assertions. It compares a minimal Seiva service against equivalent FastAPI
and Django services on the same trivial workload, with no database, so the
measurement isolates *framework overhead*.

The harness that produces these numbers lives in
[`benchmarks/`](https://github.com/ngolacloud/seiva/tree/main/benchmarks) —
`cd benchmarks && ./run.sh`. Everything below is reproducible.

> **Read the caveats.** These figures are environment-dependent and were
> measured on one shared developer machine over loopback. They show the right
> order of magnitude and the structural gap; they are **not** a universal
> "Seiva is N× faster" headline. Re-measure on your own hardware before you
> quote a ratio.

## What was measured

Three services, each exposing the **same two endpoints with identical logic**
and **no database** (an in-process atomic counter stands in for the
repository — the Seiva scaffold's in-memory default, mirrored in Python):

- `GET /health` — routing + serialize a tiny JSON body.
- `POST /v1/notes` — parse `{"title"}`, reject an empty title with `422`,
  assign a monotonic id, echo `{"id","title"}` with `201`.

| Service | Runtime | Server |
|---|---|---|
| Seiva | Rust, compiled `--release` | tokio multi-thread (all cores), built-in |
| FastAPI | CPython 3.12, interpreted | uvicorn (uvloop+httptools), 20 workers |
| Django | CPython 3.12, interpreted | gunicorn WSGI, 20 sync workers |

Two Seiva security controls were switched off for the run so the comparison is
like-for-like with the no-auth Python apps: the per-IP **rate limiter** (its
default 300 req/60s would answer a load generator almost entirely `429`) and
the **auth guard** on `POST /v1/notes`. Both are security features, not
request-processing speed; see `benchmarks/README.md` for the full rationale.

## Results

Measured 2026-11-05. **Numbers apply only to this configuration:**

- Host: 20 logical CPUs, 13th Gen Intel(R) Core(TM) i9-13900H, Linux.
- Load: `oha` 1.16.0, **50 connections, 20 s measured, 5 s warmup discarded**,
  HTTP/1.1 keep-alive, over **loopback** (generator shares CPU with the server).
- Versions: FastAPI 0.115, uvicorn 0.32, Django 5.1, gunicorn 23, Python 3.12.
- Python workers: 20 (= CPU count) for both FastAPI and Django.

### GET /health (trivial liveness; no body)

| Framework | Requests/sec | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| **Seiva (Rust)** | **271,132** | **0.148** | **0.415** | **0.708** |
| FastAPI (uvicorn) | 79,128 | 0.488 | 1.542 | 2.738 |
| Django (gunicorn WSGI) | 17,086 | 2.485 | 5.622 | 7.929 |

### POST /v1/notes (validate non-empty title, assign id, echo JSON)

| Framework | Requests/sec | p50 (ms) | p95 (ms) | p99 (ms) |
|---|---:|---:|---:|---:|
| **Seiva (Rust)** | **249,475** | **0.163** | **0.443** | **0.779** |
| FastAPI (uvicorn) | 51,763 | 0.772 | 2.230 | 3.749 |
| Django (gunicorn WSGI) | 16,217 | 2.647 | 5.788 | 8.107 |

All runs completed with essentially zero non-2xx responses (a few dozen
requests out of several million were aborted at the measurement deadline).

### Reading the numbers

On this box, in this configuration:

- On the trivial `GET /health`, Seiva served about **3.4×** the throughput of
  FastAPI and about **16×** that of Django.
- On the JSON `POST /v1/notes`, Seiva served about **4.8×** FastAPI and about
  **15×** Django.
- The tail-latency gap is larger than the throughput gap: Seiva's p99 stayed
  below **0.8 ms** on both endpoints, while FastAPI's p99 was **2.7–3.7 ms**
  and Django's **7.9–8.1 ms**.

This is the expected shape of "compiled native code vs. an interpreted runtime
with a GIL": the framework overhead per request — not the business logic, which
here is nil — is what differs.

## Honest caveats

- **Loopback, shared host.** The load generator runs on the same machine and
  competes for CPU. A dedicated client over a real NIC would move the absolute
  numbers. Treat the figures as directional.
- **Micro-benchmark.** This isolates framework overhead on a near-empty
  handler. Real services spend most of their time in the database and
  downstream calls; in those services the *relative* framework gap shrinks
  because the framework is a smaller slice of the total. The point here is the
  floor: how little Seiva itself adds (sub-millisecond p99).
- **Worker count and versions matter.** Python throughput scales with worker
  processes (20 here); the ratios would change with a different count or newer
  releases. Always report the configuration with the number.
- **Not a universal ratio.** "~3–5× vs FastAPI, ~15× vs Django" is *this
  machine, this workload*. Re-run `benchmarks/run.sh` to measure yours.

## Reproduce it

```bash
cd benchmarks
./run.sh                                   # default: 50 conns, 20s, 5s warmup
CONNECTIONS=100 DURATION=30 WARMUP=10 ./run.sh
```

Results land in `benchmarks/results/<timestamp>/` (table, raw `oha` JSON per
endpoint, and each server's log). See
[`benchmarks/README.md`](https://github.com/ngolacloud/seiva/tree/main/benchmarks)
for the methodology and fairness rules.

---

*Part of the [Seiva documentation](index.md). See also
[Performance](topics/performance.md) and the
[Competitive landscape](competitive-landscape.md).*
