# Competitive landscape — where Seiva (and DORM) win, and where we must improve

An honest scorecard of Seiva against **FastAPI** (+ fastapi-cli) and **Django**,
and of **DORM** against **Prisma**, with the gaps we must close to deliver value
and compete. Written to be used, not to flatter.

## Positioning in one line

> **FastAPI's developer experience + Django's completeness — as a compiled,
> no-GC, single distroless binary.** FastAPI is a routing+validation library;
> Django is batteries-included but on Python; Seiva aims to be batteries-included
> *and* native.

## Scorecard

Legend: ✅ strong · 🟡 partial/in-progress · ✖ missing.

Legend for the Seiva column: a ⬆︎ marks a cell that moved in the P0–P1 campaign
(see the [progress delta](#progress-delta) below for the proof).

| Capability | FastAPI | Django | **Seiva** | DORM / Prisma |
| --- | --- | --- | --- | --- |
| Auto OpenAPI + live docs UI | ✅ | ✖ | ✅ faithful, **project-aware** ⬆︎ | — |
| Type/schema-driven validation | ✅ Pydantic | 🟡 forms | ✅ `serde` + `Validate` | — |
| Request→contract fidelity | ✅ | n/a | ✅ `WireSchema` (no envelope/placeholder) | — |
| Async, high throughput | ✅ (GC) | 🟡 (GIL/GC) | ✅ **no GC, Rust** | — |
| Dependency injection / ports | ✅ `Depends` | 🟡 | ✅ trait ports | — |
| Dev CLI (`dev`/reload) | ✅ fastapi-cli | ✅ runserver | ✅ `seiva start --watch` ⬆︎ | — |
| Project/app scaffolding | ✖ | ✅ | ✅ `new` + **auto-wire** ⬆︎ | — |
| ORM / data layer | ✖ | ✅ | ✅ DORM round-trip proven ⬆︎ | ✅ Prisma |
| Migrations | ✖ | ✅ | ✅ `seiva db init/dev/migrate` ⬆︎ | ✅ migrate |
| Built-in auth (verified JWT/OIDC) | 🟡 primitives | ✅ sessions | ✅ deny-by-default, verified | — |
| Object-level authz (BOLA) | ✖ | 🟡 | ✅ `Scoped<T>` compile-time | — |
| Security-by-default | 🟡 | ✅ | ✅ headers/authn/rate-limit/OWASP | — |
| Admin GUI | ✖ | ✅ **signature** | ✅ `/admin` + `seiva db studio` ⬆︎ | 🟡 Studio (data GUI) |
| Schema DSL + studio | — | — | ✅ schema sync + `db studio` ⬆︎ | ✅ Prisma |
| DB introspection (brownfield) | ✖ | 🟡 inspectdb | ✅ `seiva db pull` ⬆︎ | ✅ `db pull` |
| Multi-database | via SQLAlchemy | ✅ | 🟡 PG live; Seiva DSN-agnostic, gated on DORM backend | ✅ incl. Mongo |
| Single-binary / distroless deploy | ✖ | ✖ | ✅ **built-in** | ✅ (vs Prisma npm+codegen) |
| Hot reload / editor DX maturity | ✅ | ✅ | 🟡 hot-reload done; editor ext + compile times WIP | — |
| Docs (Diátaxis) | ✅ excellent | ✅ **gold standard** | 🟡 structure mirrors Django; content WIP | ✅ |
| Ecosystem / mindshare | ✅ huge | ✅ huge | ✖ **nascent** | ✅ huge |

## Progress delta

The P0–P1 campaign that followed the first scorecard. Every ⬆︎ cell above points
to a slice here, and every slice closed only against a fitness test or a
reproducible live demo (so the moves are earned, not asserted). Branches are
stacked and unpushed; the owner integrates.

| Gap | What landed | Proof | Commit |
| --- | --- | --- | --- |
| Standalone compile | generated project builds via interim `[patch.crates-io]`; main uses `router_with_modules` | fitness compiles a generated project + module | `d145961` |
| Contract fidelity | `WireSchema` trait+derive+inventory; body = DTO directly, real schemas | `seiva openapi` → `{title}`/`{id,title}`, 0 placeholders | `feceb67` (ADR 0007) |
| Dev CLI reload | `seiva start --watch` / `--no-watch` | unit-tested `resolve_reload` | `561b437` |
| Facade discovery | `seiva module new` auto-wires; `seiva openapi` documents the project | fitness runs the bin → `/v1/widgets` | `a5ab52e` |
| Data layer | DORM gateway on published-crate deps; module round-trip | live `Accounts::create_user` → persist+outbox+relay vs PG | `dbdd93a`, `06dd60b` |
| Brownfield introspection | `seiva db pull` (delegates `dorm db pull`) | live → faithful `model CatalogItem` from a PG table | `f203cba` |
| Migrations | `seiva db init/dev/migrate`; module schema → migration | live → `public.records` from the `notes` model | `006b9a8`, `82a7418` |
| Admin GUI | `/admin` console (from the contract) + `seiva db studio` (data) | live `/admin` 200; `db studio` served rows | `2da7c64`, `c5fe05f` (+ DORM `552eaaf`) |

**Remaining** (the honest frontier): **P0-4** publish the crates (owner-only) —
drops the interim patches; **P1-8** multi-database — Seiva's data commands are
DSN-agnostic, but a *live* second backend is gated on DORM's runtime backend
(MySQL is a compiler dialect profile in the current checkout; the live backend is
DORM roadmap, ADR 0011); **P2** — docs content, a VS Code extension, a published
benchmark, and project-aware `seiva proto`.

## Where we are genuinely better

1. **Native performance, no GC, one binary.** Against both Python frameworks this
   is the structural win: higher throughput, lower tail latency, no interpreter/GIL,
   and a distroless, non-root, shell-less image out of the box (the
   `seiva-cloud-native-check` discipline). Deploy is `COPY one-binary`, not "ship a
   Python/Node runtime + deps".
2. **Contract fidelity by construction.** The OpenAPI document is *generated from
   the same contract the server enforces*, with real schemas (`WireSchema`, ADR
   0007) — no drift between docs and reality. FastAPI gets this from Pydantic; we
   match it and extend it to permissions (`x-seiva-requires`) and link-time
   inventory (every mounted facade is listed — OWASP API9).
3. **Security that the type system enforces.** Deny-by-default verified JWT/OIDC
   (RS256/ES256 only, never `alg:none`), and **object-level authorization as a
   compile-time concern** (`Scoped<T>`). Django is secure-by-default at runtime;
   Seiva pushes key guarantees to compile time.
4. **Architecture you cannot violate.** The five-layer dependency rule is the crate
   graph; fitness functions fail the build on violations. This is a correctness and
   maintainability story neither FastAPI nor Django enforces.
5. **DORM's native-Rust thesis is validated by Prisma's retreat.** Prisma spent
   years on a Rust query engine and in Prisma 7 (Nov 2025) removed it — because the
   **Rust↔JS FFI boundary** was the bottleneck. DORM is native Rust end-to-end: no
   FFI, no GC, compile-time type safety through to the binary. We should say this
   out loud.

## Where we must improve (prioritised)

> The plan to close every gap below — with verifiable done-criteria — is the
> [roadmap](roadmap.md).

**P0 — table stakes to be taken seriously vs FastAPI/Django:**

1. **Ship the data layer (DORM cutover).** "ORM optional, cutover WIP" is the single
   biggest credibility gap vs Django and the main reason to choose a *framework*.
   Finish the `dorm` gateway, migrations UX, and a first-class "model → repository"
   path. *This is the #1 value unlock.*
2. **Hot-reload dev loop.** `fastapi dev` / `runserver` set the bar: edit-and-see.
   `seiva start` needs watch+rebuild (and fast incremental builds). Rust compile
   times are the DX tax; invest in it (cargo-watch integration, a thin reload).
3. **Runtime facade discovery for `seiva openapi`/`proto`.** Today the CLI documents
   only the built-in demo facade; it must introspect the user's project so the
   generated contract is the project's.
4. **Publish the crates to crates.io.** Standalone compile works via an interim
   `[patch.crates-io]`; real adoption needs `cargo add seiva-*`. (See
   [publishing](publishing.md).)

**P1 — the "batteries" that define the Django category:**

5. **An admin / auto-CRUD surface.** Django's signature feature; no competitor
   matches it. Even a generated read/update API + a minimal web UI from the facade
   descriptors would be a differentiator (we already have the descriptors).
6. **DB introspection (brownfield).** `db pull`-style generation lowers adoption
   friction enormously — critical for teams with an existing schema.
7. **Migrations maturity** — drift detection, reversibility, a shadow-DB dev loop
   (Prisma's `migrate dev` is the reference).
8. **Multi-database breadth** beyond PostgreSQL (MySQL at least; the roadmap exists).

**P2 — reach and trust:**

9. **Documentation content.** The *structure* now mirrors Django (this tree); the
   gold-standard bar is *filled* tutorials + reference. Keep writing; it is the
   cheapest adoption lever.
10. **Editor/tooling DX** — a VS Code extension story (schema + facade
    autocompletion) to rival Prisma's and FastAPI's.
11. **Ecosystem & examples** — real-world templates (the "full-stack" template that
    pulled FastAPI adoption), integrations, a benchmark we publish.
12. **Scaffold quality polish** — `Vec<u8>`/`format:byte` in schemas, proto
    fidelity (ADR 0007 follow-up), and the `seiva-dx-parity` "first day" checklist.

## The three bets to press in go-to-market

1. **"Native, not interpreted."** Target teams on FastAPI/Django hitting
   performance, tail-latency or cloud-cost walls. Lead with the single-binary,
   no-GC, distroless story + a published benchmark.
2. **"Honest, enforced contracts."** Sell correctness: the docs match the server,
   authorization is compile-time, the architecture can't rot. This is a CTO/staff
   argument Python frameworks can't make.
3. **"DORM: the ORM Prisma wished it could ship."** Native Rust, no FFI boundary —
   the exact thing Prisma removed. A standalone data layer that also stands alone.

---

*See also [Seiva at a glance](getting-started/seiva-at-a-glance.md) and the
[engineering log](progress.md) for the honest current state. This analysis draws on
FastAPI, fastapi-cli, Prisma ORM (incl. the Prisma 7 Rust-free release, Nov 2025)
and Django as of 2026-11.*
