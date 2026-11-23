# ADR 0044 — Native Postgres RLS as a fail-closed backstop under application-level scoping

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0004 (the DORM data-layer gateway), ADR 0019 (fail-closed authorization), ADR 0030 (the data layer is pluggable)

## Context

Seiva enforces tenant isolation **in the application**: a `Scope` is threaded
through every use case and the repository adapters filter by `tenant_id`. That is
correct and it is where the policy belongs — but it is a *single* line of defence.
One repository method that forgets the `WHERE tenant_id = $1`, one hand-written
query, one new adapter that does not apply the scope, and tenant A reads tenant B's
rows, silently. The application-level scope is only as strong as the most careless
query in the codebase.

A database that supports **Row-Level Security** can be a second, independent line:
even a query with no tenant predicate returns **zero rows** unless the current
session has declared its tenant. delonix-meet already runs this pattern — a
`tenant_tx` that issues `SET LOCAL app.tenant_id = …` and `FORCE ROW LEVEL
SECURITY` policies on every tenant table — and calls it a fail-closed backstop.
This ADR lifts that into Seiva as a **data-layer option**, explicitly as
*defence-in-depth under* the app scope, never as a replacement for it.

The forces:

- **Backstop, not substitute.** The application `Scope` stays the primary,
  portable mechanism (it works for in-memory doubles and any data layer, ADR 0030).
  RLS is the *second* line that catches what the code misses. Removing the app
  scope and "relying on RLS" is rejected: it would couple correctness to one
  database and move policy out of the domain.
- **Fail-closed by construction.** The whole value is that a *missing* predicate
  yields *no* rows. The policy must be `USING (tenant_id = current_setting(...))`
  so that if the session GUC is unset, the comparison is null and **nothing**
  matches — deny-by-default (ADR 0019), at the storage layer.
- **The owner must not escape it.** Plain RLS is bypassed by the table owner and by
  `BYPASSRLS` roles. `FORCE ROW LEVEL SECURITY` makes the policy apply even to the
  owner, so an ORM connecting as the owning role is still constrained.
- **The scope must reach the GUC.** The data adapter has to bind the request's
  `Scope` to `SET LOCAL app.tenant_id`/`app.user_id` **inside the transaction**, so
  the setting lives exactly as long as the transaction and never leaks across a
  pooled connection. This is the one seam the framework must own.
- **Session-less jobs need an audited bypass.** A background job (ADR 0034), a
  migration, a cross-tenant admin report runs without a request `Scope`. It must
  use an **explicit, audited** elevated path, not an accidental unset GUC — the
  bypass is a deliberate, logged decision, not a hole.

## Decision

Add **native Postgres RLS as a data-layer option** — part of the DORM/sqlx
adapters (ADR 0004/0030), off by default, enabled per deployment — specified as a
backstop beneath the application `Scope`.

- **Per-transaction scope binding.** Every scoped transaction opens with
  `SET LOCAL app.tenant_id = $tenant` (and `app.user_id` where a policy needs it),
  derived from the request `Scope`. `SET LOCAL` ties the GUC to the transaction, so
  a returned pooled connection carries no residual tenant.
- **Generated policies.** Migrations for tenant-scoped tables generate, per table:
  `ALTER TABLE t ENABLE ROW LEVEL SECURITY; ALTER TABLE t FORCE ROW LEVEL
  SECURITY;` and a policy `USING (tenant_id = current_setting('app.tenant_id')::uuid)`
  (with a matching `WITH CHECK` for writes). A tooling pass verifies every
  tenant-scoped table actually has its policy — a table without one is a build
  failure, not a silent gap.
- **Fail-closed semantics.** With no `app.tenant_id` set, `current_setting('app.tenant_id', true)`
  is null, the predicate is null, and the query returns zero rows and writes none —
  the backstop is deny-by-default (ADR 0019).
- **Explicit, audited bypass for session-less work.** A `with_rls_bypass(reason)`
  helper runs on a role permitted to set a cross-tenant context (or `BYPASSRLS`),
  and **every** use records an audit entry (ADR 0043) with the reason and the
  caller. Jobs, migrations and cross-tenant reports go only through this path.
- **Opt-in per data layer.** RLS is a property of the Postgres-backed adapters; the
  in-memory doubles and non-Postgres layers keep app-scope-only. Enabling RLS
  changes no application code — it is a deployment and migration choice (ADR 0030's
  pluggability).

The policy — *which* tables are tenant-scoped and the tenancy model — is the
product's; the mechanism — the GUC binding, the generated policies, the forced RLS,
the audited bypass — is the framework's.

## Alternatives considered

- **App-level scoping only (today).** Rejected as sufficient for high-assurance
  deployments: it is correct but single-layered; one forgetful query is a
  cross-tenant leak with no backstop. RLS adds an independent second line.
- **RLS only, drop the app scope.** Rejected: it welds tenant correctness to one
  database, loses portability across the pluggable data layers (ADR 0030) and the
  in-memory doubles, and moves policy out of the domain into DDL. Defence-in-depth
  wants *both*, not a swap.
- **Plain RLS without `FORCE`.** Rejected: the owning role (which the ORM commonly
  uses) bypasses non-forced RLS, so the backstop would not apply to the very
  connection that needs it. `FORCE ROW LEVEL SECURITY` is mandatory.
- **A per-tenant schema or database.** Rejected as the default: schema-per-tenant
  scales poorly to many tenants and complicates migrations and pooling; a single
  schema with a tenant column + forced RLS is the pragmatic backstop. (It remains a
  legitimate choice for a few very large tenants — orthogonal to this ADR.)

## Consequences

- **Performance.** `current_setting` is a cheap per-transaction read; the
  `tenant_id` predicate rides existing indexes (every tenant-scoped table indexes
  `tenant_id`), so RLS adds a constant-factor filter, not a scan; `SET LOCAL` is one
  statement per transaction.
- **Security.** **Defence-in-depth** — RLS catches any query the application scope
  fails to constrain; `FORCE ROW LEVEL SECURITY` means **not even the table owner**
  escapes; **fail-closed** — an unset GUC yields zero rows (ADR 0019); the only way
  past it is an **explicit, audited** bypass (ADR 0043) for session-less work.
- **Resilience.** The backstop is **independent of the application code's
  correctness**: a future refactor, a new adapter, a hand-written query cannot
  reintroduce a cross-tenant leak while RLS is enabled, so isolation does not
  degrade as the codebase grows.
- delonix-meet's `tenant_tx`/`FORCE RLS` pattern becomes a framework option any
  deployment can switch on, and every platform handling multiple tenants' data
  gains a storage-level isolation guarantee under its application scope.
