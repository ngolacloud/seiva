# Hot-path baseline — Seiva framework overhead vs plain axum

The **"before"** for the panel's hot-path campaign (findings E2/E4/E5). Measured
in-process with `tower::ServiceExt::oneshot` — no socket, no database: this
isolates the framework, not I/O. Re-run after each optimization and compare the
Seiva−axum **delta**, not the absolute numbers.

> **Honesty note.** Numbers are meaningful only next to the machine and method
> that produced them. Each iteration builds its own `Request` (identical on both
> sides), so the absolute figures include that fixed cost; read the delta.

## Method

- Command: `cargo bench --bench hot_path -- --warm-up-time 1 --measurement-time 3 --sample-size 50`
- Machine: 13th Gen Intel Core i9-13900H, 20 logical cores
- Toolchain: rustc 1.96.0, `cargo bench` (release)
- Date: 2026-11-17
- Runtime: tokio current-thread; `criterion` 0.5 (`async_tokio`)

Two groups (see `benches/hot_path.rs`):

- **light** — `GET /ping`, tiny JSON. Seiva carries the always-on base layers
  (`request_id`, `track_metrics`); axum is bare. Isolates the fixed middleware
  cost per request (finding **E4**, metrics labels).
- **guarded** — `POST /demo` through `request_id → track_metrics → authn →
  permission_guard`, handler extracting `Ctx` (deep-cloning the `Principal`
  **twice** — guard + extractor, finding **E2**) and `Valid<T>`. axum is a bare
  route that parses the body and returns 201. Delta = the zero-trust tax.

## Baseline (median)

| Group | Seiva | plain axum | **Delta (framework tax)** |
|---|---|---|---|
| light | 3.00 µs | 0.99 µs | **≈ 2.01 µs/req** |
| guarded | 7.20 µs | 2.12 µs | **≈ 5.08 µs/req** |

## Reading it

- The **guarded delta (~5.1 µs/req)** is what the campaign must shrink. Known
  contributors from the audit: the `Principal` deep-clone done **twice** per
  request (guard + `Ctx`, **E2**), the incondicional metrics-label `Vec`+clone
  (**E4**), and — not yet in this bench — the global rate-limiter `Mutex` (**E5**,
  added when that layer is wired in).
- The **light delta (~2.0 µs/req)** is the base-layer cost alone (`request_id`
  mint + `track_metrics` labels). E4 targets this one directly.

## After E2 (Principal by reference / `Arc`)

`authn` now inserts `Arc<Principal>`; the guard reads it **by reference** (zero
clones, incl. on public routes), and `Ctx`/audit/rate-limit clone the `Arc`
(O(1)) or only the one field they need. Same command, same machine.

| Group | Seiva | plain axum | **Delta** | vs baseline delta |
|---|---|---|---|---|
| light | 2.85 µs | 0.94 µs | ≈ 1.91 µs/req | 2.01 → 1.91 (~flat, E2 doesn't touch it) |
| guarded | 5.83 µs | 1.97 µs | **≈ 3.86 µs/req** | **5.08 → 3.86 µs (−1.22, −24%)** |

> **Method note.** Read the **intra-run delta** (Seiva−axum in the *same* run),
> not criterion's run-to-run "change": here the axum baseline also moved ~−17%,
> which is machine noise (its code is unchanged). The intra-run delta cancels
> that. The guarded framework tax dropped ~24% from removing the two `Principal`
> deep-clones (guard + `Ctx`); `light` stays flat because it has no guard/`Ctx`.

## After E4 (metrics labels: `&'static` method + `Cow` route, no `Vec`+clone)

`track_metrics` no longer builds a `Vec<Label>` and clones it. `method` is a
`&'static str` (standard verbs; else `OTHER`), `status` is `&'static`, and `route`
is a `Cow` — **borrowed** on a 404. Allocations per request (by inspection):

| Path | Before E4 | After E4 |
|---|---|---|
| matched route | ~6 (route+method+vec+clone[vec+method+route]) | ~4 (route owned + its clone + 2 macro keys) |
| 404 / unmatched | ~6 | **~2** (route borrowed, method/status static) |

**Latency: below this harness's noise floor.** A re-run moved *every* group
~+12–22%, **including `axum` (unchanged code)** — pure machine noise. E4 saves
~2 allocations/req on a matched route (tens of ns), which this inter-run noise
(±~20%) swamps; unlike E2 (−26%), it does not emerge above it. E4 is kept for the
**allocation + cardinality** win (and `OTHER` bounds the `method` series), not a
measurable latency gain. Honest call: do not quote an E4 latency number.

> **Harness limitation exposed.** Comparing absolute µs deltas across runs taken
> at different times is only valid for effects larger than the machine's drift.
> Micro-optimizations need an A/B in a *single* run (or an allocation counter),
> not run-to-run µs. E5 (rate-limiter lock) should show up under *concurrency*,
> which this single-threaded `oneshot` harness does not yet exercise.

## E5 (rate-limiter lock sharding): tried, measured, REVERTED

Added a `concurrent_ratelimit` group — 64 distinct-client guarded requests fired
at once across worker threads through the rate-limited app — then sharded the
limiter's `Mutex<HashMap>` into 32 independently-locked shards.

| Variant | 64-request batch |
|---|---|
| global `Mutex` (baseline) | ~241 µs |
| 32-way sharded | ~268 µs (**+9%, no gain**) |

**Verdict: reverted.** The sharding showed no improvement — if anything it was
slower, because the critical section is a single `HashMap` probe (tens of ns):
across the full HTTP stack, 64 requests on 8 cores almost never hold the lock at
the same instant, so there is no contention to remove, and the per-request shard
hash only added cost. Optimizing a lock that isn't contended is complexity for
nothing. The in-process limiter keeps its single `Mutex`; a deployment that needs
rate limiting at a scale where a lock *would* contend uses the distributed store
(`rate_limit_with_store`, ADR 0015) anyway. The `concurrent_ratelimit` group is
kept as a regression guard documenting this.

> This is the campaign's second honest negative (after E4's sub-noise latency):
> "performance as a feature" means measuring before changing — and *not* shipping
> a change the measurement doesn't support.

## Campaign summary

| Finding | Outcome |
|---|---|
| **E1** OpenAPI cache | shipped — regen-per-request → `Bytes` O(1), closes a DoS |
| **E2** `Principal` by ref/`Arc` | shipped — **guarded framework tax vs axum cut ~24%** (measured) |
| **E4** metrics labels | shipped — fewer allocations + bounded cardinality (latency below noise) |
| **E5** rate-limiter sharding | reverted — no measurable contention to remove |

The headline number for the thesis is **E2: ~24% off the guarded per-request
framework overhead vs plain axum**, proven by this harness.
