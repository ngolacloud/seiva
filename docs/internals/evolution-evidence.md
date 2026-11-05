# Evolution campaign — evidence by SHA, matrices and honest gaps

The evolution of Seiva into a cloud-native backend framework with the admin
console as an extension (the owner's six-phase brief). Baseline: `main` at
`aed02d1f`. Work landed on branch **`fix/ci-green-baseline`** (not pushed —
owner's PR decision). Every commit was verified locally; the DORM-coupled and
live-database proofs ran against a real PostgreSQL 16.

## Commits (in order)

| SHA | Phase | What | Proof |
|---|---|---|---|
| `3800bbd` | F1 | CI green: `Problem` → `Box<str>` (128→88 B), blanket `result_large_err` allows removed (gate restored), DORM-gateway `fmt` | 5 CI jobs reproduced green locally |
| `c82cf67` | F2 | Admin extracted to optional `seiva-console-admin`; base `/admin`→404; hash-based CSP (no `unsafe-inline`) — ADR 0008 | base 404 test; `cargo tree -p seiva-server` = 0 occurrences; CSP-hash matches served bytes |
| `2322ca1` | F3a | Per-module `AdminResource` registry; console shows only registered **and** authorized resources — ADR 0009 | unit + integration (Principal-filtered registry); duplicate→conflict |
| `240816a` | F3b | `seiva module new --admin` scaffolds the registration | scaffold test; `--admin` present/absent |
| `dc2a509` | F3c | `seiva ext add` + declarative `seiva-extension.toml` integration contract — ADR 0010 | live scaffold→`ext add` idempotent; manual-patch fallback |
| `f28f666` | F3 fix | Security review (OWASP): mount extensions INSIDE the framework layers so `/admin` gets authn/headers/rate-limit | end-to-end test: `Principal` flows through layers to the registry |
| `156d6e7` | F4 | Durable `IdempotencyStore` port + DORM adapter (reuses `dorm_core::idempotency`) — same-tx with mutation+outbox — ADR 0011 | **live PG**: 20 concurrent connections → 1 effect; Mismatch; Replay bytes |
| `679f7ac` | F4 | HTTP `Idempotency-Key` extractor + scoped-key + canonical fingerprint + 409 mapping | unit + end-to-end (Fresh once / Replay / 409 / 400) |
| `b7f6755` | F4 | Consumer inbox dedup (`inbox_key`) + TTL `purge_expired` worker (`run_idempotency_purge`) | **live PG**: redelivery→1 consume; ttl=0 purged→reclaimable |
| `5505053` | F5 | Health probes exempt from the rate limiter; cached readiness dependency; liveness ignores deps — ADR 0012 | probe-flood stays 200; dep down→503 ready/200 live; cache probed once/TTL |
| `d7c9c27` | F5 | Persistent admin-mutation audit (`AuditSink`) with secret redaction; DORM sink — ADR 0013 | unit (metadata, denied-too, reads-skipped, redaction) + **live PG** (password `[redacted]`, secret never stored) |
| `b0737fa` | F5 | WireSchema unit-enums → string enum; fidelity tests (nested/array/bytes/Option); OpenAPI Problem error responses (401/403/422) — ADR 0007 addendum | derive + openapi tests |
| `7c37b67` | §9 | `cargo-deny` advisories for the excluded DORM-coupled lockfiles; 3 real advisories surfaced + tracked — ADR 0014 | `cargo deny` green for all 5 excluded crates with reasoned, dated exceptions |
| `5828149` | §9 | Tuned LLVM release profile (ThinLTO, `codegen-units=1`, strip), no `target-cpu=native` | measured: `seiva` 6334→1680 KiB (−73%), build +31% |

New ADRs: **0008–0014** (+ the 0007 addendum). Indexed in
[ADRs](adrs.md).

## Acceptance-criteria matrix

| Requirement (§10) | Where | State | Evidence |
|---|---|---|---|
| Backend builds/boots/tests with Cargo, no DX/admin | workspace | ✅ | `cargo test --workspace` green; no DX in the graph |
| `/admin` → 404 with no extension; no admin in `cargo tree` | `seiva-http`, `seiva-server` | ✅ | base 404 test; tree = 0 occurrences |
| Extension install is explicit, no hooks, needs recompile | `seiva ext add`, ADR 0010 | ✅ | live proof; declarative manifest, no build.rs |
| Repeated install duplicates nothing | `add_extension_*`/`mount_*` | ✅ | idempotency tests |
| Module `--admin` → listed/authorized registration | ADR 0009 | 🟡 | registration + authz-filtered registry done; list/detail/form **UI** over data rows pending |
| Module without registration not exposed | registry gate | ✅ | absent from registry → absent from console |
| Other tenant / no permission → no access | registry filter + `Scoped<T>` (ADR 0005) | ✅ | registry hides unauthorized; BOLA guard on the API |
| Repeated admin request → one effect across replicas | idempotency (ADR 0011) | ✅ | **live PG** 20-connection proof |
| Core coherent after extraction | docs/tests | ✅ | base tests + docs aligned |
| Security: audit of all lockfile variants + negative tests | ADR 0014, deny gates | 🟡 | advisories gate extended to excluded lockfiles; license/bans on excluded crates deferred |
| Quality: fmt, clippy, unit/integration, doctests | CI | ✅ | all green each commit |
| E2E + load with thresholds | `e2e/admin` (browser) | 🟡 | admin browser E2E passes on headless Chrome (authenticate → list authorized → create, no secret leak); edit/filter/version-conflict + production load benchmark still pending |

## Compatibility matrix

| Component | Version / state | Depends on | Notes |
|---|---|---|---|
| Seiva base (`seiva-*`, `delonix-seiva`) | 0.1.0 | — (DORM only behind `dorm` feature) | DB-agnostic; ports only |
| `seiva-console-admin` | 0.1.0 (opt-in extension) | Seiva base public API (`axum`, `delonix-seiva::admin`, `seiva-identity`) | one-way; base never depends on it (ADR 0008) |
| DORM | `dorm-core 0.6`, `dorm-outbox 0.1` (git/path, pre-publish) | — | behind the `dorm` feature / excluded crates; idempotency + audit adapters live here |
| DX | external tool (`projects/dx`) | reads `seiva-extension.toml` (ADR 0010) | not required at compile/runtime; Cargo equivalent is `seiva ext add` |

## Honest gaps (remaining)

- **F5 tail.** Distributed rate limiter — **done** (ADR 0015): `RateLimitStore`
  port + `rate_limit_with_store` + optional `seiva-ratelimit-redis`, proved live
  against Redis 7 (50 concurrent clients share one budget). Console field-level
  masking of data rows remains (the API path has `Scoped<T>`/BOLA; row-level
  masking is module-handler work, tied to the console's model-row CRUD).
- **F6 tail.** Browser E2E of the admin (Playwright) — needs a headless browser
  and a running server. Production load benchmark with auth + idempotency +
  multiple replicas (p50/p95/p99, throughput, errors, CPU, RSS) — needs that
  infra; the published microbenchmarks remain overhead-only and are labelled so.
- **gRPC** is proto-generation only; no served round-trip is claimed, so none is
  asserted (ADR 0007 addendum).
- **Admin console** is an **operations** console (contract-driven forms), not a
  model-row CRUD Django-admin; registration + server-side authorization gate are
  done, generic data browsing/editing is a later increment.

Nothing above is a silent gap: each is stated here and in the relevant ADR.
