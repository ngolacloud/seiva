# ADR 0009 — Per-module admin registration and the authorization gate

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0005, ADR 0006, ADR 0007, ADR 0008

## Context

ADR 0008 extracted the admin console into the optional `seiva-console-admin`
extension. That first cut rendered **every** operation in `/openapi.json`: being
in the contract was enough to appear in the console. Two problems:

1. **Exposure is not authorization (OWASP API9/API1).** The linker including a
   facade must not put it in an operator UI. A module should decide, explicitly,
   whether it is administrable, and under which permissions.
2. **No module-owned metadata.** Labels, list columns, searchable/orderable
   fields and per-operation permissions have nowhere to live. Django modules
   declare this in an `admin.py`; Seiva had no equivalent.

Constraints from the owner's brief:

- A module that opts in must not take a dependency on the admin **UI** crate
  (`seiva-console-admin`): disabling the extension must leave no mandatory
  dependency in a module's domain/application layers. "Register-and-forget" must
  not couple the module to the console.
- The console must show a resource only when it is **registered** *and* the
  caller is **authorized** to view it. `tenant_id` from a request body must never
  decide access; the server, holding the authenticated `Principal`, decides.
- A duplicate registration (two modules claiming one `(namespace, resource)`)
  must fail loudly, at build or startup, not silently shadow one another.

## Decision

**A module registers an `AdminResource` descriptor in the framework crate's
link-time inventory via `delonix_seiva::register_admin!`. The console reads the
registry, filters it by the caller's `Principal` permissions, and shows only
registered *and* authorized resources. The descriptor type lives in
`delonix-seiva`, not in the console crate.**

- `delonix_seiva::admin` owns `AdminResource` (stable `(namespace, resource)` id,
  `label`, backing `facade`/`version`, `list_fields`/`search_fields`/`filters`/
  `ordering`/`page_size`, and `AdminPermissions { view, create, change, delete }`)
  plus `registered_admin_resources()` and `admin_registry_conflicts()`. It reuses
  the exact `inventory` mechanism facades use (ADR 0001/0007), so no hand-kept
  list exists.
- A module's **interface** layer calls `register_admin!(AdminResource { … })`.
  Because the descriptor is in `delonix-seiva` — a crate the interface layer
  already depends on for `#[controller]`/`#[facade]` — registering adds **no**
  dependency on `seiva-console-admin`. With the console unmounted, the
  registration is inert link-time data (no runtime cost, no behaviour). A module
  that does not register is **private to the console**.
- `seiva-console-admin` serves `GET /admin/registry.json`, built from
  `registered_admin_resources()` filtered server-side: a resource appears only
  when its `view` permission is non-empty **and** the request's `Principal`
  (from the `authn` middleware, defaulting to anonymous) holds it. Each
  operation's permission is resolved to a `can` flag so the UI hides what the
  caller may not do — while the module's `#[controller]` guard remains the
  authority on every call (defence in depth). The console UI renders only the
  resources the registry returns, pairing each with its `/openapi.json`
  operations by resource segment.
- **Conflict detection.** `admin_registry_conflicts()` returns duplicated ids;
  the registry endpoint returns `500` (listing them) rather than serving a
  shadowed set, and `seiva_console_admin::check()` lets a composition root turn a
  duplicate into a startup error.

The permission the console gates on is the *same string* the facade method
declares in `#[requires(..)]` (ADR 0006) — one source of truth for authorization.

### Not in this ADR (later increments)

- `seiva module new --admin` scaffolding the `register_admin!` call — Phase 3b.
- A DX `dx add seiva-console-admin` integration protocol + the Cargo equivalent —
  Phase 3c.
- Generic model-row CRUD (list/detail/edit of data rows), field-level masking and
  tenant-scoped data reads in the console — later, with the data-layer admin.

## Consequences

**Positive.**

- Exposure is now an explicit, per-module decision; the console never surfaces a
  resource just because it is in the contract.
- Authorization is enforced server-side against the authenticated `Principal`,
  with the same permission strings the API guards use.
- Modules declare admin metadata without depending on the admin UI; the extension
  stays optional and one-way.
- Double-registration fails loudly.

**Negative / cost.**

- A module must add a small `register_admin!` block to appear (until 3b
  scaffolds it). This is the intended "explicit, auditable" trade-off.
- The registry is link-time: a tool that does not link a module does not see its
  resources (the same property facades have). The runtime binary that mounts the
  modules sees the complete set, which is what hosts the console.

## Alternatives considered

- **Descriptor type in `seiva-console-admin`.** Rejected: a registering module
  would then depend on the admin UI crate, violating "disabling the extension
  leaves no mandatory dependency".
- **Derive visibility from `/openapi.json` + a permission convention.** Rejected:
  that keeps "in the contract ⇒ in the console" and offers no place for labels,
  columns or per-operation permissions.
- **A runtime `AdminRegistry` built by a `register_admin(&mut registry)` call in
  the composition root.** Viable, but it reintroduces a hand-maintained central
  list; the link-time inventory matches how facades already self-register and
  needs no central wiring.

## Addendum (2026-11-05) — the data-layer admin backend (row CRUD)

The first slice of the "model-row CRUD" follow-up is built and proved, keeping the
rule above (the console drives **contract operations**, never direct table
access, so domain invariants stay in the module):

- `apps/admin-data-e2e` is a compact DORM-backed CRUD module — a `Catalog` facade
  with all five operations (`create`/`get`/`list`/`update`/`delete`) over a real
  PostgreSQL table via `DormUnitOfWork` — that registers an `AdminResource`
  (`catalog.read/create/change/delete`) and mounts the console behind a token
  authenticator. The DORM coupling makes it an excluded crate (built by the
  token-gated CI jobs, audited by `deny.excluded.toml`).
- A live test (`live_catalog_full_crud_round_trip`, gated on `SEIVA_DORM_TEST_DSN`)
  proves create → get → list (ordered) → update → delete each persist through
  DORM against real PostgreSQL.

The console's row-CRUD **UI** is built too: a resource exposing a `list`
operation renders an admin **table** (columns from `list_fields`) with a create
form, per-row **edit** (an inline form from the `update` schema, prefilled) and
**delete** (confirm → `delete`), all through the module's contract operations.
A browser E2E (`e2e/admin/catalog.mjs`) drives the full loop against the live
`Catalog` module on PostgreSQL in a real headless Chrome — create → it appears in
the list → edit → delete → it is gone — and caught a real singular/plural
resource-matching bug.

Still pending: tenant-scoped row filtering and field masking (a `Ctx`/Principal
seam into the module), and pagination/search/ordering controls wired from the
registered allow-lists.
