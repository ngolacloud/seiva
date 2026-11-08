# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Delonix Seiva** is a Rust backend framework (the service/distribution/domain layer around a data source). A project is built from **modules that own their data**; the same module code runs as a modular monolith or as a microservice by composition alone. It is a cargo workspace of `seiva-*` crates plus `examples/` reference modules. The default (optional) data layer is [DORM](https://github.com/ngolacloud/dorm), a separate private product.

## Commands

The CI gate (`.github/workflows/ci.yml` → `check`) is the source of truth — run these before considering work done:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -q -p seiva-cli --bin seiva -- doctor   # CLI smoke test
```

- **Single test:** `cargo test -p <crate> <substring>` (e.g. `cargo test -p shop-orders-application place`). Test a single crate: `cargo test -p seiva-http`.
- **Supply-chain gate** (`audit` job): `cargo deny check` and `cargo deny --all-features check` (config in `deny.toml`).
- **Docs gate** (`docs` job, intra-doc links are errors): `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items`.
- **`seiva` CLI** (dev without installing): `cargo run -p seiva-cli --bin seiva -- <cmd>` — key commands: `project new <name>`, `module new <name>`, `module split <name>` (monolith→microservice), `start`, `openapi`, `proto`, `lint` (architecture fitness functions), `db init`/`db dev`, `user create`.

### The documentation site (`docs/`, MkDocs)

Bilingual **EN + pt_AO**, served at <https://ngolacloud.github.io/seiva/> (CI `pages.yml` deploys on push to `main`).

```sh
pip install mkdocs-material pymdown-extensions mkdocs-static-i18n
mkdocs build     # EN at site/, pt at site/pt/   (mkdocs serve to preview)
```

- English is the base; a `foo.pt.md` sibling translates `foo.md` (mkdocs-static-i18n, `suffix` structure), untranslated pages fall back to EN.
- **pt_AO uses PRE-AO90 orthography** (`acção`, `directo`, `facto`, `óptimo`, `projecto`…) — owner requirement. Keep code, identifiers, commands and relative link targets byte-identical when translating; do not add `.pt` to link targets.
- Nav section titles live in `mkdocs.yml` and are translated there via the `i18n` plugin's `nav_translations`.

## Architecture — the parts that span files

**The DORM boundary is the central structural fact.** The framework core never depends on DORM. DORM enters only through the `seiva-data-dorm` gateway behind the **`dorm` cargo feature**; the dependency is one-way (Seiva → DORM, never the reverse). The DORM-coupled crates (`seiva-data-dorm`, `seiva-outbox-relay`, `seiva-rag-worker`, `seiva-mcp`, `apps/dorm-e2e`, `apps/admin-data-e2e`) are **`exclude`d from the workspace** (see `Cargo.toml`) so the whole default build is DORM-free and needs no credentials. They are built only by the **token-gated** `dorm-gateway` / `dorm-e2e` CI jobs, which are a green no-op unless a `DORM_TOKEN` secret is present. **Do not add these crates to the default workspace members and do not make core crates depend on them.**

**A module is five crates, one per layer** (`examples/notes`, `examples/accounts`, `examples/polls`, `examples/shop/<module>` are the canonical shapes — copy them). The dependency rule is enforced by the crate graph and checked by `seiva lint` (ADR 0003):

- `facade` — the versioned contract: a `#[seiva::facade(version="v1")]` trait whose methods carry `#[requires("perm")]`, plus the wire DTOs (`#[derive(WireSchema)]`). May depend only on the shared kernel + the Seiva macros.
- `domain` — entities and rules; constructed via `new()` so a rule is always enforced. Depends on nothing but the shared kernel.
- `application` — the use-case service implementing the facade, generic over **ports** from `seiva-ports` (`UnitOfWork`, `IdGenerator`, `EventPublisher`, `Clock`) plus a repository port it defines. It must **not** name `delonix_seiva` directly, must be transport- and database-free, and unit-tests against the in-memory doubles in `seiva-testing`.
- `infrastructure` — adapters for the ports (in-memory by default; DORM-backed behind the feature).
- `interface` — the transport shell; `#[seiva::controller]` generates the facade-driven `axum` router. Must not depend on `infrastructure` or `domain`.

**The contract is the single source of truth.** From one facade trait, a compile-time descriptor drives the HTTP routes + permission guard (`#[seiva::controller]`), the OpenAPI 3.1 document (`seiva-openapi`, served at `/openapi.json`), and the gRPC proto (`seiva-grpc`). No reflection — everything is known at compile time. Contract fidelity: DTO fields are published to OpenAPI via `WireSchema` (ADR 0007).

**Use cases are one transaction each, and modules cooperate through events, not calls.** A write use case runs its body inside `UnitOfWork::transaction` and publishes its domain event *inside* that transaction (with DORM, into a transactional outbox, so data and event commit atomically). Modules never import each other's crates; the order flow (`orders.placed` → `inventory.reserved` → `payments.captured` → `notifications.sent`) is wired by the composition root reacting to events. This independence is what makes `seiva module split` a composition change, not a rewrite.

`seiva-server` is the runtime that boots a service (HTTP app + `/openapi.json` + `/health` + worker host + graceful shutdown, 12-factor config). `seiva-cli` scaffolds/lints/runs. The architecture decisions are recorded in `docs/adr/`; roadmap state in `docs/progress.md`.

## Conventions and gotchas

- **Adding a module/example = adding its five crates to the `members` list in the root `Cargo.toml`.** Copy the layering and macro usage from an existing `examples/` module.
- **Do not name a DTO `Send`** (or any std marker trait): the application ports bound futures `+ Send`, so a type called `Send` collides and fails to compile — use a qualified name (e.g. `SendNotification`).
- `WireSchema` supports primitives, `Vec<T>`, `Option<T>` and nested derived structs (`crates/delonix-seiva/src/schema.rs`).
- Commits follow **Conventional Commits** (Portuguese or English subject). Branch for feature work; `main` is the integration branch (trunk-based, merged via PR).
- The CI jobs `dorm-e2e` / `dorm-gateway` build the excluded DORM-coupled crates against the **published** `dorm-core`/`dorm-outbox` from crates.io (no `DORM_TOKEN`, no local path patch — PR #20), so they are green on this repo like the rest. The gates that matter for a DORM-free change are still `check`, `audit`, `docs`; if you reintroduce a local `[patch.crates-io]` to an excluded `apps/*` crate, these jobs go red without the token.
- Project-scoped skills live in `.claude/skills/` and are worth running when relevant: `seiva-pattern-fitness` (before adding a module/changing a layer), `seiva-security-review` (OWASP API Top 10 for `seiva-http`/`seiva-auth`/`seiva-webhooks`), `seiva-cloud-native-check` (distroless image proof), `seiva-dx-parity` (CLI/HTTP ergonomics), `seiva-community-adr` (write an ADR before a new pattern/boundary).
