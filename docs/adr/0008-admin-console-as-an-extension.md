# ADR 0008 — The admin console is an extension, not part of the base

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0003, ADR 0005, ADR 0006, ADR 0007

## Context

Seiva grew an admin console as roadmap item P1-5: `admin.rs` inside
`seiva-http`, served at `GET /admin` by `framework_routes`, so **every** Seiva
backend — the module-free skeleton included — shipped the console's HTML/CSS/JS
and answered `200` on `/admin`. The page is a self-contained, CDN-free operations
UI that reads the service's own `/openapi.json` (ADR 0007) and renders a try-form
per write operation, exercised with a bearer token.

Three problems with that placement:

1. **Boundary.** The framework base is meant to be a publishable HTTP building
   block with no opinion about operator UX. Baking an admin UI into `seiva-http`
   means a backend that wants no admin surface still compiles, ships and serves
   it — the console's assets are in the binary and the route is live. There is no
   way to get a backend with *no* admin code; `cargo tree` always shows it
   because it *is* the transport crate.

2. **Security posture.** The console's `Content-Security-Policy` used
   `script-src 'unsafe-inline'` and `style-src 'unsafe-inline'` — the base
   framework's own default security headers are strict, but the admin route
   widened them for the whole page. `unsafe-inline` defeats the point of a CSP:
   any injected `<script>` runs.

3. **Honesty.** Shipping `/admin` in the base reads as "Seiva has a Django-style
   admin out of the box". It does not: this first cut drives the module
   *operations* the contract declares, not generic model-row CRUD, and it has no
   per-module registration, server-side field masking or per-operation
   authorization of its own.

The forces: the console is genuinely useful and already generated purely from the
live contract (no per-module code), so it should keep working; but the base must
be able to build, boot and serve with **no** administrative code at all; and the
dependency must run one way only — an extension may depend on the base's public
API, never the reverse, and the base must not grow a `default = ["admin"]`
feature that silently re-couples them.

## Decision

**Extract the console into a separate, optional crate `seiva-console-admin`, an
*extension* of the Seiva base. The base (`seiva-http`) carries no admin code and
answers `404` for `/admin`. A composition root opts the console in explicitly by
calling `seiva_console_admin::mount(router)` and recompiling; nothing in the base
depends on the extension.**

Concretely:

- `seiva-http` loses `mod admin`, the `admin_handler`/`ADMIN_HTML` re-exports and
  the `/admin` route in `framework_routes`. Its surface is now exactly health,
  `/openapi.json` and `/docs`. A regression test asserts `/admin` is `404`.
- `seiva-console-admin` is a workspace member (so CI builds and tests it) that
  depends only on `axum` (the base's public transport type) plus `sha2`/`base64`
  for the CSP hashes. **No base crate depends on it**, so a backend built without
  it has no admin assets and no `/admin` route — provable with `cargo tree`.
- The crate exposes `admin_router() -> Router`, `mount(Router) -> Router` and
  `ADMIN_PATH`. `mount` is the explicit, auditable opt-in; a second `mount`
  panics at router-build time (axum rejects a duplicate route), so a double-mount
  is a startup error, never a silent one.
- **CSP hardening.** The inline `<style>` and `<script>` are pinned by their
  SHA-256 hash (`script-src 'sha256-…'; style-src 'sha256-…'`), computed at
  runtime from the exact bytes served, so the policy can never drift from the
  payload. `unsafe-inline` is gone. The policy adds `base-uri 'none'`,
  `form-action 'none'` and keeps `default-src 'none'`, `connect-src 'self'`,
  `frame-ancestors 'none'` and `img-src 'self' data:` — no remote origin anywhere.

### Not in this ADR (later increments)

This ADR moves the existing console and hardens its CSP. It does **not** add:

- Per-module registration (`seiva module new --admin`, an `AdminRegistry`, typed
  forms, list/filter/search/ordering/pagination, custom actions) — Phase 3.
- Server-side authorization *by the console* (per-operation/object/tenant/field),
  beyond the guards each module's `#[seiva::controller]` already enforces on the
  routes the console calls — Phase 3/5.
- A DX `dx add seiva-console-admin` integration protocol — Phase 3.

Those are tracked separately so this extraction can land as a complete,
verifiable step.

## Consequences

**Positive.**

- A backend with no admin builds, boots and serves with no administrative code;
  `/admin` is `404` and `cargo tree` shows no `seiva-console-admin`.
- The base framework is a clean transport building block again; the console
  evolves on its own cadence without touching `seiva-http`.
- The admin page no longer weakens its CSP with `unsafe-inline`.
- The dependency edge is one-way and explicit: the extension depends on the
  base's public API; the base never depends on the extension.

**Negative / cost.**

- Existing projects that relied on `/admin` being present by default must add the
  crate and a one-line `mount(..)` call (documented in the migration guide).
- The console's assets are duplicated out of `seiva-http` history into the new
  crate (the move preserves the code; `git` records the deletion + addition).
- A composition root must recompile to gain the console — by design: a declared
  dependency alone does not mount routes (ADR follows the "explicit composition"
  rule of ADR 0003).

## Alternatives considered

- **Feature flag `seiva-http/admin` (default off).** Rejected: it keeps the admin
  code and assets inside the base crate (still shipped source, still a maintenance
  coupling) and invites a `default = ["admin"]` regression. An extension crate
  makes the boundary physical.
- **Keep `/admin` in the base, only harden the CSP.** Rejected: fixes the CSP but
  not the boundary or the honesty problem; a no-admin backend is still impossible.
- **Per-request nonce instead of hashes.** A nonce needs a CSPRNG per response and
  template injection; for a fully static page a hash over the served bytes is
  simpler and has no runtime randomness dependency. Revisit if the page gains
  request-specific inline content.
