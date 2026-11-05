# Seiva vs FastAPI vs Django — benchmark harness

A reproducible micro-benchmark that backs one specific claim honestly: that a
Seiva service, being **compiled native code**, serves a trivial request with
less per-request overhead than the same trivial service on an interpreted
Python framework. It measures the *framework*, not a database, not business
logic, and not network I/O.

> **Honesty note.** Numbers are meaningful only next to the hardware and
> method that produced them, and they are environment-dependent. This README
> is the method; measured results live in `results/<timestamp>/results.md`
> after you run `./run.sh`, and a committed snapshot (with its caveats) is in
> [`../docs/benchmarks.md`](../docs/benchmarks.md). Do not quote a number
> without the machine it came from.

## What is compared

Three services, each exposing the **same two endpoints with the same logic**:

| Endpoint | Behaviour (identical across all three) |
|---|---|
| `GET /health` | Return `200` with a tiny JSON body. Pure routing + serialize. |
| `POST /v1/notes` | Parse `{"title": "..."}`, reject empty/whitespace title with `422`, assign a process-local monotonic id, echo back `{"id": N, "title": "..."}` with `201`. |

There is **no database**. Each app's "repository" is an in-process atomic
counter — this is deliberate: a DB would dominate the timing and hide the
framework overhead we are trying to isolate. The Seiva scaffold already ships
an in-memory repository + counter id-generator, so the Python apps mirror that
exactly.

### The three apps

- **Seiva (Rust, compiled):** `notesbench/`, scaffolded with the Seiva CLI
  (`seiva project new` + `seiva module new notes`). It is its **own standalone
  cargo workspace** with a `[patch.crates-io]` pointing at the local framework
  checkout — it is **not** a member of the main repo workspace, so building it
  never touches the framework build. Built with `cargo build --release`.
- **FastAPI (Python, interpreted):** `fastapi_app/main.py`, served by
  **uvicorn** with `--workers N` (uvloop + httptools from `uvicorn[standard]`).
- **Django (Python, interpreted):** `django_app/app.py`, a single-module WSGI
  app served by **gunicorn** with `N` sync workers — the usual production
  posture for Django.

## Fairness rules applied

- **Release / production servers only.** Seiva is `--release`; Python runs
  under uvicorn/gunicorn, not the dev autoreloader.
- **Equal endpoint semantics.** Same validation, same status codes
  (`201` on create, `422` on empty title), same JSON shape.
- **Warmup before measuring.** A discarded warmup pass precedes every measured
  pass so JIT-less Python import costs and cold caches don't skew the first
  samples.
- **Same load profile** applied to all three (connections, duration).
- **Multi-core for everyone.** Seiva's tokio runtime uses all cores; the
  Python servers are given `WORKERS = nproc` processes so they, too, can use
  the whole machine. (Override `WORKERS` to explore scaling.)

### Two deviations, declared

1. **Seiva's rate limiter is turned off for the run.** The scaffold default
   rate-limits each client IP to 300 requests / 60s. A load generator hammers
   from one IP, so leaving it on would make Seiva answer almost everything
   `429` — measuring the rate limiter, not request throughput. It is a security
   control, not a speed feature, and the Python apps have no equivalent, so the
   benchmark build sets `rate_limit: None` (see
   `notesbench/apps/notesbench-api/src/main.rs`).
2. **Seiva's auth guard is removed on `POST /v1/notes`.** The scaffold marks it
   `#[requires("notes.create")]`; with no authenticator wired that route
   answers `403`. The FastAPI/Django apps have no auth layer, so the guard is
   removed in the benchmark scaffold (`modules/notes/facade/src/lib.rs`) to
   compare like with like.

Both deviations are documented in-code at the edit site.

## How to run

Prerequisites: `cargo`, `python3` (3.10+), and the [`oha`](https://github.com/hatoo/oha)
load generator (`cargo install oha`). `run.sh` creates `./.venv` and installs
the Python deps itself.

```bash
cd benchmarks
./run.sh                      # default profile
CONNECTIONS=100 DURATION=30 WARMUP=10 ./run.sh    # heavier
ONLY=seiva,fastapi ./run.sh   # subset
WORKERS=4 ./run.sh            # fewer Python workers
```

Each run writes to `results/<timestamp>/`:

- `results.md` — the throughput + p50/p95/p99 table with the host/profile header,
- `raw.tsv` — machine-readable rows,
- `<fw>_<endpoint>.json` — the raw `oha` output for audit,
- `<fw>.server.log` — the server's own log.

Config knobs (env vars): `CONNECTIONS` (default 50), `DURATION` seconds
(20), `WARMUP` seconds (5), `WORKERS` (nproc), `SEIVA_PORT`/`FASTAPI_PORT`/
`DJANGO_PORT` (18080/81/82), `ONLY` (comma list of frameworks).

`run.sh` starts each server in turn, benchmarks it, then stops it and frees the
port (it traps EXIT and `fuser -k`s the ports), so nothing is left bound.

## Caveats you must keep attached to any number

- **Loopback, shared host.** The load generator runs on the same machine and
  competes with the server for CPU. Absolute throughput would differ with a
  dedicated client over a real NIC. Treat results as *directional*.
- **Micro-benchmark.** It isolates framework overhead on a trivial handler.
  Real services spend most of their time in the database and downstream calls,
  where the framework gap shrinks in relative terms. The point here is only the
  floor: how much the framework itself adds.
- **Worker count matters.** Python throughput scales with `WORKERS`; the
  default `nproc` is a generous, fair setting on this box. Report it alongside
  the number.
- **Versions matter.** Record the Seiva, FastAPI, Django, uvicorn, gunicorn and
  oha versions with any published figure (the harness prints them).
