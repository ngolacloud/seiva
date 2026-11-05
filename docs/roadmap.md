# Roadmap — closing the P0–P2 gaps

This plan turns the gaps in the [competitive landscape](competitive-landscape.md)
into concrete, **verifiable** workstreams. The rule: a gap is *closed* only when
its scorecard cell flips from 🟡/✖ to ✅, backed by a fitness function or a
reproducible demo — so the [re-comparison](#re-running-the-comparison) at the end
is objective, not a matter of opinion.

Effort is T-shirt sized (S ≈ a day, M ≈ a few days, L ≈ 1–2 weeks, XL ≈ a month+).

## Dependency map (what gates what)

```
P0-4 publish crates ─────────────┐
P0-3 runtime facade discovery ───┤ (independent quick wins)
P0-2 hot-reload dev loop ────────┘

P0-1 DORM cutover (XL) ──┬─▶ P1-7 migrations maturity
                         ├─▶ P1-6 DB introspection
                         ├─▶ P1-8 multi-database breadth
                         └─▶ P1-5 admin / auto-CRUD ─▶ (reads descriptors + data)

P2-9 docs content ───────── (continuous, unblocks adoption)
P2-10 VS Code ext · P2-11 examples+benchmark · P2-12 scaffold/proto polish
```

Start the three P0 quick wins in parallel with P0-1 (the long pole). P1 mostly
waits on P0-1. P2 runs continuously.

---

## Phase 0 — table stakes (be taken seriously vs FastAPI/Django)

### P0-1 · Ship the data layer (DORM cutover) — **XL**

- **Goal:** a generated module can persist through DORM, not just the in-memory
  adapter — the single biggest credibility gap vs Django.
- **Approach:**
  1. Bring the `seiva-data-dorm` gateway into a buildable state behind the `dorm`
     feature (it is currently workspace-excluded because it needs the private DORM
     git dep + CI token — decide: vendored path, published DORM crate, or a
     token'd CI job).
  2. Provide a DORM-backed `RecordRepository` adapter in the module `infrastructure`
     template, selected in the composition root when `dorm` is on.
  3. Wire `UnitOfWork`/transactions and the outbox through the gateway.
  4. Map the schema DSL (`schema/<project>.schema` `namespace` blocks) to DORM
     models end to end.
- **Done (verifiable):**
  - A fitness test scaffolds a project + module **with `--data dorm`**, runs it
    against a real Postgres (testcontainer or the live PG:55432 harness), and
    `POST`→`GET` round-trips a record through DORM. rc 0.
  - `seiva doctor` reports the DORM data layer linked.
  - Scorecard: *ORM/data layer* 🟡→✅.
- **Depends on:** DORM repo access decision (owner).

### P0-2 · Hot-reload dev loop (`seiva start --watch`) — **M**

- **Goal:** edit-and-see parity with `fastapi dev` / `runserver`.
- **Approach:** add a file-watcher (notify/cargo-watch-style) to `seiva start` that
  rebuilds and restarts the `apps/<name>-api` process on change; debounce; stream
  logs; keep the port. Invest in incremental build speed (workspace `profile.dev`
  tuning, `-Zthreads`/`cranelift` where available) — compile time is the DX tax.
- **Done:** `seiva start --watch` restarts on a `.rs` change in < a few seconds for
  an incremental build; a scripted demo edits a handler and shows the new response
  without a manual restart. Scorecard: *dev CLI* 🟡→✅.
- **Depends on:** nothing.

### P0-3 · Runtime facade discovery for `seiva openapi`/`proto` — **S/M**

- **Goal:** the CLI documents the *project's* facades, not just the built-in demo.
- **Approach:** the runtime already discovers every mounted facade via the
  link-time inventory (`registered_facades`, used by `/openapi.json`). Give the CLI
  a path that reads the *project's* compiled facades — either (a) have `seiva openapi`
  invoke a tiny generated binary in the project that prints the document from the
  project's own inventory, or (b) emit the document from the running `seiva start`
  server (`GET /openapi.json`) and have the CLI fetch/dump it.
- **Done:** in a scaffolded project with a module, `seiva openapi` outputs that
  module's operations (not `NotesFacade`); the stderr limitation note is removed. A
  fitness test asserts the generated doc contains the project's route. Scorecard:
  *(contributes to)* the OpenAPI story becomes project-true.
- **Depends on:** nothing (builds on WireSchema/ADR 0007 already done).

### P0-4 · Publish the framework crates to crates.io — **S** (owner-gated)

- **Goal:** `cargo add delonix-seiva seiva-http …` works; drop the interim
  `[patch.crates-io]` from generated projects.
- **Approach:** the crates are publish-ready (metadata, version deps). Publish in
  dependency order (`seiva-ports` → `seiva-macros` → … → `seiva-cli`). Then change
  the scaffold to stop appending the patch once a published version exists; keep the
  patch path as an opt-in dev override (`SEIVA_FRAMEWORK_DIR`).
- **Done:** a generated project compiles with **no** `[patch.crates-io]` section
  (resolving from crates.io); the standalone-compile fitness runs in that mode.
  Scorecard: *single-binary/standalone* fully ✅ for end users.
- **Depends on:** owner (publishing is owner-only); see [publishing](publishing.md).

---

## Phase 1 — the batteries that define the Django category

### P1-5 · Admin / auto-CRUD surface — **L** (the signature differentiator)

- **Goal:** the one thing no FastAPI/Prisma competitor matches — auto-CRUD from the
  model/contract. We already have the facade **descriptors**; use them.
- **Approach (incremental):**
  1. Generate a read/write **admin API** from the facade descriptors + DORM models
     (list/get/create/update/delete per model), guarded by an `admin.*` permission.
  2. A minimal, self-hosted **web UI** (server-rendered or a small embedded SPA)
     that drives that API — list views, detail/edit forms generated from the
     `WireSchema` of each DTO.
  3. Register-and-forget: `seiva module new` makes a module admin-visible by default.
- **Status (2026-11-05):** an **operations** console (a form per contract
  operation, bearer token) exists and is now extracted to the optional
  `seiva-console-admin` extension (ADR 0008) — the base returns `404` for
  `/admin`, the console mounts explicitly, and its CSP is hash-based (no
  `unsafe-inline`). Still **pending** (the register-and-forget model-CRUD vision
  above): per-module `--admin` registration, read/write admin API over DORM
  models, server-side field/tenant authorization in the console, and the
  Playwright E2E. Scorecard: *Admin GUI* 🟡 (operations console), not yet ✅.
- **Depends on:** P0-1 (data), P0-3 (discovery).

### P1-6 · Database introspection (brownfield adoption) — **M/L**

- **Goal:** `db pull`-style generation of the schema from an existing database — the
  biggest adoption-friction reducer for teams with a schema already.
- **Approach:** a `seiva db pull` (DORM introspection) that reads `information_schema`
  and emits `namespace`/model blocks; then scaffold modules around them.
- **Done:** against a seeded Postgres, `seiva db pull` produces a schema that
  `seiva db migrate` round-trips with zero drift. Scorecard: *introspection* 🟡→✅.
- **Depends on:** P0-1.

### P1-7 · Migrations maturity — **M**

- **Goal:** match Prisma's `migrate dev`/`deploy` reference workflow.
- **Approach:** drift detection (shadow DB), reversible migrations, a clear dev
  (`seiva db dev`) vs deploy (`seiva db migrate`) split, migration history integrity.
- **Done:** a fitness test: change a model → `seiva db dev` generates a migration →
  apply → detect no drift → roll back. Scorecard: *migrations* 🟡→✅.
- **Depends on:** P0-1.

### P1-8 · Multi-database breadth — **M/L**

- **Goal:** beyond PostgreSQL — MySQL at least (the DORM multi-backend roadmap
  already exists).
- **Approach:** enable the DORM MySQL backend through the gateway; a backend matrix
  in CI.
- **Done:** the DORM round-trip fitness (P0-1) passes against MySQL too. Scorecard:
  *multi-database* 🟡→✅ (for PG+MySQL).
- **Depends on:** P0-1.

---

## Phase 2 — reach and trust (continuous)

### P2-9 · Fill the documentation content — **L (continuous)**

- **Goal:** move the "planned"/"draft" pages (this tree already mirrors Django) to
  "published" — the cheapest adoption lever.
- **Done:** every tutorial part and every topic index is written and runnable; a
  link/anchor check and a "no stub left in getting-started/topics" fitness pass.

### P2-10 · VS Code extension — **M**

- **Goal:** schema + facade autocompletion/validation to rival Prisma's and FastAPI's.
- **Done:** an extension that highlights the schema DSL and surfaces facade/`requires`
  diagnostics; published to the marketplace (or VSIX).

### P2-11 · Examples, templates and a published benchmark — **M**

- **Goal:** a real-world template (the thing that pulled FastAPI adoption) + a
  reproducible benchmark backing the "native, not interpreted" claim.
- **Done:** a `seiva project new --template <x>` that scaffolds a non-trivial app;
  a committed benchmark harness + numbers vs FastAPI/Django in `docs/benchmarks.md`.

### P2-12 · Scaffold / proto / DX polish — **S/M**

- **Goal:** close the known small gaps.
- **Done:** proto3 fidelity from `WireSchema` (the ADR 0007 follow-up);
  `Vec<u8>`→`format:byte` in composed schemas; the `seiva-dx-parity` "first day"
  checklist green.

---

## Milestone sequence (suggested)

- **M1 (quick credibility):** P0-3 + P0-4 + P0-2 — the project documents itself, the
  crates are installable, and the dev loop feels like FastAPI. (Parallel, ~2–3 weeks.)
- **M2 (the data layer):** P0-1 — the long pole. Unblocks all of P1.
- **M3 (batteries):** P1-7 + P1-6 + P1-8, then P1-5 (admin) as the headline.
- **M4 (reach):** P2-9 continuous throughout; P2-10/11/12 as capacity allows.

## Re-running the comparison

When the phases land, re-run the scorecard objectively:

1. Each closed gap must point to its **fitness test or demo** (the "Done" line
   above). No cell flips to ✅ without one.
2. Re-measure the three go-to-market bets with evidence: the **benchmark** (P2-11)
   backs "native, not interpreted"; the **admin + DORM round-trip** back
   "batteries-included"; the **contract-fidelity + compile-time authz** tests back
   "honest, enforced contracts".
3. Update [competitive-landscape.md](competitive-landscape.md) in place and diff the
   table against this baseline — the delta is the value delivered.

---

*Part of the [Seiva documentation](index.md). Baseline:
[competitive landscape](competitive-landscape.md) (2026-11).*
