# ADR 0005 — Object-level authorization (BOLA / API1): fail-closed by type

- Status: Accepted
- Date: 2026-11-04
- Deciders: NgolaCloud
- Follows: ADR 0003

## Context

Seiva already enforces **function-level** authorization (OWASP API5): the
`permission_guard` middleware reads the permission a route requires from the
facade's `MethodDescriptor` and refuses a caller who lacks it. That answers "may
this caller call this operation at all".

It does **not** answer **object-level** authorization (OWASP API1, Broken Object
Level Authorization — BOLA): "may this caller touch *this specific* resource".
BOLA is the most common and most damaging API vulnerability precisely because
function-level checks pass — the caller is a legitimate user with the right
permission — while the object belongs to another tenant. A route middleware
cannot answer it: the middleware runs *before* the handler loads the resource,
so it does not yet know which tenant owns it.

The framework shipped `Principal::owns` / `Principal::require_owns` as a building
block for this. But they are **advisory**: nothing forces a handler to call them.
A handler that loads a record and returns it, forgetting `require_owns`, serves
one tenant's data to another — silently, with a `200`, past every existing gate.
The gap is a matter of developer discipline, which is exactly what a reference
framework should remove, not rely on.

The forces:

- The check needs the resource's tenant, so it can only happen *after* the load
  — at the application/handler seam, not at the edge.
- "Forgetting the check" must not be a silent, fail-open mistake.
- The layering rule (ADR 0003) forbids `interface → domain` and
  `application → delonix-seiva`/transport, so wherever the primitive lives
  constrains who can build it.

## Decision

**Make the unchecked path unrepresentable with a tenant-tagged wrapper,
`Scoped<T>`, whose inner value is reachable only by passing the ownership
check.** `require_owns`/`owns` stay as the explicit escape hatch.

1. `Scoped<T>` carries a resource's `tenant` alongside its value. It is built
   where the resource is loaded, from the record's own tenant column:
   `Scoped::new(record.tenant, dto)`.

2. The **only** way to read the value is
   `Scoped::<T>::owned_by(self, &Principal) -> Result<T, Problem>`, which returns
   the `T` iff the principal owns the tenant and a `403` `Problem` otherwise.
   There is no `into_inner`, no public field, no `Deref`. A handler that forgets
   to check therefore cannot reach the resource to return it — the omission is a
   **compile error**, not a runtime leak. `Scoped<T>` is `#[must_use]`, so even
   dropping it on the floor warns.

3. `Principal::require_owns`/`owns` remain for the cases a wrapper does not fit —
   e.g. a list already filtered by `WHERE tenant = $1` in SQL, where the tenant
   boundary was enforced at the query and there is no per-item value to unwrap.
   Using them is then a deliberate, visible assertion, not a silent default.

## Alternatives considered

- **Keep `require_owns` advisory only (status quo).** Rejected: it fails open. A
  forgotten call is invisible — no type error, no test failure, no CI signal —
  and BOLA is too costly to leave to discipline.

- **An object-level route middleware, like `permission_guard`.** Rejected as
  impossible, not merely undesirable: object identity is unknown until the
  handler has loaded the resource, so a middleware necessarily runs too early to
  know the tenant. Function-level and object-level checks live at different
  points in the request's life by nature.

- **Declare the object policy in the descriptor and enforce it in the generated
  controller, reading a tenant off the response DTO.** Rejected *for now*: it
  forces every tenant-owned DTO to expose its tenant through a `HasTenant` trait,
  couples the transport layer to a domain tenancy concept, and still cannot cover
  resources that never cross the wire. A heavier contract change for narrower
  coverage than the wrapper. Left as a possible future layer on top of `Scoped`.

- **`Scoped<T>` — the type-level capability (chosen).** The smallest change that
  flips the failure mode from open to closed, needs no descriptor or macro
  change, reads clearly to an outside developer, and composes with the existing
  `Principal`.

## Consequences

- **Easier:** a reviewer who sees a `Scoped<T>` reach a response knows it passed
  the ownership check — the type says so. The BOLA gap becomes a compile error a
  developer hits while writing the handler, not a leak found in production.

- **Harder / accepted debt (2026-11-04):** adoption is per-module and opt-in —
  a module only gains the guarantee once its use case/handler threads
  `Scoped<T>`. Modules returning raw DTOs stay on the honour system until they
  adopt it. Tracked in `docs/progress.md`.

- **Layering follow-up (2026-11-04):** `Scoped<T>` and `Principal` live in
  `seiva-http` today, so the wrapper is built at the **interface/handler seam**
  (from the DTO's tenant), not inside `application` — a use case returning
  `Scoped<T>` directly would pull transport into `application`, which ADR 0003
  forbids. Moving the ownership primitive into a transport-free crate
  (`seiva-auth` or a kernel) so `application` can return `Scoped<T>` itself is
  the refactor the first real multi-tenant module will force; deferred until
  then rather than guessed at now.

- **Enforcement:** the type itself is the fitness function — no `into_inner`, no
  public field, no `Deref`, `#[must_use]` on the wrapper. A future `seiva lint`
  check that flags a facade DTO carrying a `tenant` field whose handler path
  reaches a response without going through `Scoped`/`require_owns` is listed as
  follow-up work, not shipped here.
