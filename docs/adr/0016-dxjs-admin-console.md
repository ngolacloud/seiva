# ADR 0016 — The admin console may be served by a dxjs SPA

- Status: Accepted
- Date: 2026-11-06
- Deciders: NgolaCloud
- Follows: ADR 0007, ADR 0008, ADR 0009, ADR 0010

## Context

The admin console is an opt-in extension (ADR 0008): `seiva-console-admin`
serves `GET /admin` as a **single, self-contained, zero-build** page — inline
HTML/CSS/JS emitted from Rust string constants, pinned by a strict
`Content-Security-Policy` whose `script-src`/`style-src` carry the SHA-256 hash
of the exact inline bytes (no `unsafe-inline`, no `unsafe-eval`). It reads the
live contract (`/openapi.json`, ADR 0007) joined with the authorization-filtered
`/admin/registry.json` (ADR 0009) and renders tables/forms, escaping every value
and attaching handlers with `addEventListener`. Authorization is enforced
server-side by each module's `#[seiva::controller]`; the UI's `can.*` flags are
presentational.

That design is deliberately minimal. As the console grows (richer data tables,
client-side search/sort/pagination already declared in the registry, live
updates, inline editing, diagram/pipeline views), hand-written inline JS inside
Rust string literals becomes the limiting factor: no component model, no
reactivity, no tests for the browser behaviour, and every feature is more inline
string concatenation.

An independent audit (see the StillJS → Seiva report) evaluated adopting an
existing Vanilla-JS framework and **rejected** it: its rendering engine was
`eval()`-based and could not run under Seiva's strict CSP. That audit motivated a
clean-room engine, **dxjs** (`github.com/ngolacloud/dxjs`), built to the exact
constraint this console sets: fine-grained reactivity with **no `eval`, no inline
handlers, output escaped by default** — proven in a real browser under
`default-src 'none'; script-src 'self'` with zero CSP violations, and proven
end-to-end against *this* backend's live `/admin/registry.json` + `/openapi.json`
(the `admin-e2e` composition root) via a same-origin harness.

The force in tension: Seiva values the console being **self-contained and
zero-build** (ADR 0008 removed its CDN dependency on purpose), but richer admin
UX wants a real component engine. A dxjs console is an *external same-origin
module* (`<script type="module" src="/admin/dx.js">`), which needs only
`script-src 'self'` — still no `unsafe-inline`/`unsafe-eval`, so the CSP stays
strict — but it introduces a **pre-built JS bundle** the crate must ship.

## Decision

**Adopt dxjs as the console's rendering engine, introduced as an opt-in
alternative mount, not a hard replacement of the inline page.**

Concretely:

- `seiva-console-admin` keeps serving the existing inline, hash-pinned page at
  `GET /admin` as the **default and fallback** (unchanged; still zero extra
  deps, still the strictest CSP).
- A new opt-in surface serves the dxjs console (`seiva_console_admin::mount_dx`
  or a `dx` cargo feature) at a distinct route (e.g. `GET /admin/dx`), delivering
  a **committed, pre-built** dxjs bundle as an external same-origin module under
  `default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'self';
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'` — no
  `unsafe-inline`, no `unsafe-eval`, no remote origin.
- The dxjs bundle is produced from `examples/seiva-console` in the dxjs repo and
  vendored into this crate as a checked-in artifact with a recorded SHA-256; a CI
  check asserts the served bytes match the committed bundle (the ADR-0008
  "policy and payload can never drift" property, kept).
- Making dxjs the **default** `/admin` (and retiring the inline page) is a later,
  separate decision, taken only once the dxjs console reaches parity (data
  tables, search/sort/pagination, inline edit/delete, mutation audit ADR 0013)
  and the bundle pipeline below is accepted.

Authorization, registry filtering (ADR 0009) and WireSchema contract (ADR 0007)
are unchanged: dxjs consumes the same two endpoints read-only; the `can.*` flags
stay presentational; the backend remains the sole authority.

## Alternatives considered

1. **Keep the inline Rust console only (status quo).** Zero build, strictest
   possible CSP, nothing new to maintain. Rejected as the *end state* because it
   caps the console's UX: richer tables, reactivity and diagram views are
   impractical as inline string concatenation, and the browser behaviour has no
   component model or tests. Retained as the **default/fallback**, which is why
   this ADR does not remove it.

2. **Replace `/admin` with the dxjs SPA now (hard cutover).** Simplest mental
   model, one console. Rejected *for now*: it deletes a well-engineered,
   dependency-free implementation before the replacement has feature parity, and
   it forces every Seiva backend that opts into the console to also carry a JS
   bundle. A framework should not make that irreversible step before parity is
   demonstrated and the bundle pipeline is accepted. This remains the intended
   destination (see Decision), gated on a follow-up ADR.

3. **Load dxjs from a CDN.** Smallest crate. Rejected outright: it reintroduces
   the exact remote-origin dependency ADR 0008 removed and would require widening
   `script-src`/`connect-src` to a third-party origin — against the console's
   self-contained, strict-CSP principle.

## Consequences

Easier:

- The console gains a real component model, reactivity, a router and tests
  (dxjs is unit- and browser-tested), so richer admin UX (keyed data tables,
  client search/sort/pagination from the registry allow-lists, live updates via
  SSE, inline edit) becomes tractable instead of inline-string work.
- The engine is CSP-clean by construction, so the strict-CSP property Seiva
  insists on (ADR 0008) is preserved on the new route.

Harder / debt accepted (2026-11-06):

- `seiva-console-admin` stops being strictly zero-build for the `dx` surface: it
  ships a **checked-in, pre-built JS bundle** produced outside this repo (dxjs).
  Debt: the bundle must be rebuilt and re-vendored when the dxjs console changes,
  and its provenance (version + SHA-256) must be recorded. We accept this only
  for the opt-in route; the default `/admin` stays zero-build.
- A second console implementation exists until a future ADR retires the inline
  one — temporary duplication, bounded by that follow-up.
- The `dx` route uses `script-src 'self'` (external module) instead of a per-byte
  hash. This is still strict (no `unsafe-*`, same-origin only); the integrity
  guarantee moves from an inline hash to a **CI check that the served bundle
  equals the committed, SHA-256-recorded artifact**.

Enforcement:

- A CI gate in this repo asserts the served `dx.js` bytes match the committed
  bundle and its recorded SHA-256 (payload/policy drift guard, per ADR 0008).
- The dxjs repo's own CI proves the console renders under strict CSP (browser
  proof) and against this backend's live contract (the `admin-e2e` harness).
- `seiva-pattern-fitness` and `seiva-security-review` run on the crate change
  that adds the `dx` mount.

## Legibility verdict

Readable without the codebase: an outside Rust developer learns that `/admin`
stays the zero-build hash-pinned page, that a strict-CSP dxjs SPA is offered opt-in
as the path to a richer console, exactly which security properties are preserved
vs. traded (per-byte hash → committed-bundle CI check), and that a full cutover is
a deliberate later decision — not this one.
