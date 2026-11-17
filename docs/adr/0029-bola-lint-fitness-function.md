# ADR 0029 — A BOLA fitness function in `seiva lint`: tenant-scoped response DTOs must be returned scoped, so object-level authorization stops being an honour system

- Status: Proposed
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0005 (object-level authorization / BOLA), ADR 0019 (fail-closed authorization), ADR 0003 (module layering and the `seiva lint` fitness functions)

## Context

ADR 0005 gave Seiva a strong object-level authorization primitive: `Scoped<T>`
(`crates/seiva-http/src/guard.rs`) has no `into_inner`, no public field and no
`Deref`; the only way out is `owned_by(&Principal)`, which returns `403` on a
tenant mismatch, and it is `#[must_use]`. Where a module uses it, forgetting the
check is a **compile error** — BOLA closed by type.

But adoption is **opt-in and per-module**. ADR 0005 itself records the gap:
*"Modules returning raw DTOs stay on the honour system."* A new module that loads a
record by id and returns its DTO **without** `owned_by`/tenant scoping serves one
tenant's object to another with a `200`, sailing past every function-level guard
(`#[requires("perm")]` checks the *verb*, not the *object*). The framework ships
the safety primitive but **does not force its use and cannot detect its absence** —
the exact shape of the number-one API risk (BOLA, OWASP API1).

The forces:

- Seiva already treats architecture rules as **executable fitness functions**:
  `seiva lint` enforces the layer-dependency rule from ADR 0003 against the crate
  graph. BOLA adoption should be the same kind of gate — a rule the CI checks,
  not a convention the reviewer must remember.
- Full data-flow analysis ("does a `tenant` field reach a response without passing
  through `Scoped`?") is too ambitious and too fragile to run statically on real
  Rust, and a lint with false positives trains people to ignore it. The rule must
  be **cheap, local and high-signal**, built on information the framework already
  has at compile time: the facade descriptor and the `WireSchema` of each wire DTO
  (ADR 0007).
- It must stay **fail-closed** (ADR 0019): the *default* for a tenant-bearing
  response DTO returned without a scope annotation is **error**, not pass. Opting
  out must be **explicit and auditable**, never silent.
- It must not break existing modules gratuitously: the reference modules already
  scope correctly, so the rule should pass on them and only fire on the honour-
  system shape.

## Decision

Add a `bola` fitness function to `seiva lint` (the same tool that enforces ADR 0003
layering), run in CI as a hard gate.

**What it inspects.** For every facade method, the lint reads the compile-time
descriptor already produced by `#[seiva::facade]` and the `WireSchema` of each
response DTO (both exist today — no new reflection):

1. **Classify the response DTO as tenant-bearing** when its `WireSchema` (including
   nested derived structs) contains a field whose name matches a configurable set —
   default `tenant`, `tenant_id`, `owner`, `owner_id`. These are the DTOs whose
   cross-tenant exposure is a BOLA leak.
2. **Require a scope decision on the returning method.** A method that returns a
   tenant-bearing DTO must satisfy one of:
   - return it as `Scoped<T>` / a scoped collection (the type-enforced path of
     ADR 0005), **or**
   - carry an explicit `#[scoped]` attribute asserting the application layer scopes
     the query by `&Ctx` (the pattern the reference modules use, where the scope is
     derived server-side from the principal and pushed into the DORM gateway), **or**
   - carry an explicit `#[cross_tenant(reason = "...")]` opt-out — an **auditable**
     marker for a legitimately cross-tenant admin/reporting read.
3. **Default is error.** A tenant-bearing response DTO returned by a method with
   none of the three is a **lint error** that fails CI. Fail-closed: the unmarked
   case does not pass.

**Reporting.** The error names the facade, the method and the offending DTO field,
and states the three ways to resolve it — the same ergonomic bar as the layering
lint, so the fix is obvious from the message (ADR 0003's diagnostic style).

**Opt-out is auditable, not invisible.** `#[cross_tenant(reason = "...")]` is
greppable and reviewable; a security review can enumerate every deliberate
cross-tenant read in the codebase from the annotations alone.

## Consequences

- The largest structural security gap — BOLA adoption left to discipline — becomes
  a CI gate. A new module that returns a tenant-bearing DTO without scoping it
  **fails the build**, so the honour system is replaced by a fitness function,
  consistent with how ADR 0003 enforces layering.
- The cost to module authors is one explicit annotation on the rare cross-tenant
  read and nothing on the common scoped read (which already returns `Scoped<T>` or
  derives the scope from `&Ctx`). The reference modules pass unchanged.
- Security review gains an inventory: every `#[cross_tenant]` is an auditable,
  justified exception, instead of an invisible raw-DTO return nobody flagged.
- It is a **heuristic, not a proof**: field-name classification can miss a
  tenant-bearing DTO that names its discriminator unconventionally, or flag a field
  named `owner` that is not a tenant key. The configurable name set and the
  `#[cross_tenant]` escape hatch keep both error modes cheap to resolve, and the
  rule is strictly additive safety over today's zero checks.
- It does not replace `Scoped<T>` (ADR 0005) — it **drives adoption of it**: the
  lint pushes authors toward the type-enforced path, where forgetting the check is
  already a compile error.

## Alternatives considered

- **Full static data-flow analysis** (track a tenant value from load to response).
  Rejected: too expensive and too brittle on real Rust; false positives would get
  the lint disabled. The descriptor+`WireSchema` heuristic is cheap, local and runs
  on information already present.
- **A runtime assertion** (panic/`403` if a tenant-bearing DTO leaves unscoped).
  Rejected: it is not fail-closed at compile time, it only fires on the code path
  that executes (so an untested path still leaks), and it moves a build-time
  guarantee to production.
- **A custom Clippy lint.** Rejected: Clippy sees the AST, not the facade
  descriptor or the `WireSchema`, so it cannot reason about *response DTOs of
  facade methods*; `seiva lint` already owns that model (ADR 0003).
- **Leave it to code review (status quo).** Rejected as the whole reason for the
  ADR: review is exactly the honour system ADR 0005 flagged, and BOLA is the
  number-one API risk — it deserves a gate, not a reviewer's memory.
