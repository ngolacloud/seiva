---
name: seiva-pattern-fitness
description: Run seiva lint plus the Seiva anti-pattern catalogue before adding a module or changing a layer. Rejects a pattern applied outside its nature (e.g. CQRS forced on trivial CRUD, or a layer dependency the architecture forbids).
---

# Seiva pattern fitness

Enforce the framework's architecture rules on a module change. The compiler and
`seiva lint` are the first line; this skill is the catalogue of patterns that are
*technically legal* but *wrongly applied*.

For a deeper architecture judgement (DDD + Clean, module-vs-microservice, the
bootstrap, the open findings F1–F6), convene the panel in the
`seiva-architecture-reviewers` skill (Fowler + Uncle Bob + Evans) and reconcile
against the backlog `docs/audits/2026-11-architecture-review.md`.

## 1. Run the fitness function

- `cargo test -p delonix-seiva lint::` and `cargo run -p seiva-cli -- lint`
  (inside a project) — the layer-dependency and contract checks must pass.
- `cargo test -p delonix-seiva scaffold::` — the scaffold fitness suite, including
  `generated_project_compiles_standalone` (compiles the generated project with the
  module wired into `main`), `project_is_cloud_native_ready`, and the seed/demo
  checks `generated_module_has_seed_and_demo` / `composition_root_wires_seed_on_boot`
  (ADR 0022). Closing a finding means turning its fitness green, not just editing.
- The five layers and the dependency rule (`crates/delonix-seiva/src/lint.rs`):
  - `domain` → only `shared/kernel`.
  - `facade` → never DORM; never domain/application/infrastructure/interface.
  - `application` → its own domain/facade + `seiva-ports`; never the framework
    runtime, the data layer, or a lower layer.
  - `infrastructure` → application/domain/ports; never `interface`.
  - `interface` → facade + application (+ transport crates); **never
    `infrastructure`, never `domain`**. The controller is GENERIC over the
    ports; the composition root (`apps/`) injects the concrete adapters.

## 2. Anti-pattern catalogue (reject with a reason)

- **CQRS forced on trivial CRUD.** `seiva-cqrs` is opt-in. A module with a single
  create/read path does not need a `Mediator`; adding one is ceremony. Only reach
  for CQRS when commands and queries genuinely diverge (separate models,
  read-side caching, independent scaling).
- **`interface -> infrastructure` in production.** The interface wiring a concrete
  repository names the infrastructure crate and breaks the layer rule. Keep the
  controller generic; wire concretes in the composition root. (This was the real
  Part C finding; see `docs/progress.md` S4.)
- **Five layers for a config-only or read-only shim.** If a "module" has no
  domain rules and no outbound port, question whether it is a module at all or
  just a route on an existing one (the module→service escalation is for when a
  bounded context needs real isolation, not for every CRUD table).
- **A facade method with no `requires` that touches a tenant's data.** Function-
  level authz (`permission_guard`) is skipped for empty `requires`; a data path
  must declare a permission AND check ownership.

## 3. Output

List any rule violation (`file:line`) the compiler/lint did not already catch —
those are bugs in the verification. List any anti-pattern applied out of place,
each with the lighter alternative. If clean, say so explicitly.
