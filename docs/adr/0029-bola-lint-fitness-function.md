# ADR 0029 — A BOLA fitness function in `seiva lint`: a literal `Scope::Global` in a module's application layer is a build error unless it carries an auditable cross-tenant opt-out

- Status: Accepted
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0005 (object-level authorization / BOLA), ADR 0019 (fail-closed authorization), ADR 0003 (module layering and the `seiva lint` fitness functions), SG1 (`Scope` / `#[model(tenant_scoped)]`)

## Context

SG1 gave Seiva a strong tenant-isolation primitive. A data-layer session is opened
for an **explicit** [`Scope`](../../crates/seiva-ports/src/lib.rs): either
`Scope::Tenant(TenantId)` — which makes `dorm_core`'s `scope.rs` inject a
`tenant = <current>` predicate on every read and force the `tenant` column on every
insert, so a forgotten filter cannot cross tenants — or the deliberate, **audited**
`Scope::Global`, which the gateway maps onto DORM's system-scope escape (it emits a
`tracing` event). `Ctx::scope()` derives that scope **server-side** from the
authenticated principal, never from the request body (OWASP API1), so the common
path is safe by construction.

But the safe path is **opt-in per use case**. A module's application layer passes a
`Scope` to `UnitOfWork::transaction`, and nothing stops it passing a **literal
`Scope::Global`** — bypassing tenant isolation for every statement in that
transaction — by habit, by copy-paste, or because it was the first thing that
compiled. `examples/notes` did exactly this, and so do the single-tenant demo
modules. A new module that writes and reads under a hard-coded `Scope::Global`
serves one tenant's object to another with a `200`, sailing past every
function-level guard (`#[requires("perm")]` checks the *verb*, not the *object*) —
the exact shape of the number-one API risk (BOLA, OWASP API1). The framework ships
the isolation primitive but **does not force its use and cannot detect its
bypass**.

The forces:

- Seiva already treats architecture rules as **executable fitness functions**:
  `seiva lint` enforces the layer-dependency rule (ADR 0003) and the data-layer
  neutrality rule (ADR 0030) against module crates. "Don't bypass tenant scoping"
  should be the same kind of gate — a rule CI checks, not a convention the reviewer
  must remember.
- The rule must be **cheap, local and high-signal**. A full data-flow analysis
  ("does a tenant value reach a response unscoped?") is too expensive and too
  brittle on real Rust, and a lint with false positives trains people to ignore it.
  The one thing that is unambiguous, greppable and exactly the bypass we care about
  is a **literal `Scope::Global`** written into a module's application code.
- It must stay **fail-closed** (ADR 0019): the *default* for a literal
  `Scope::Global` in a module's `application`/`domain` layer is **error**. Opting
  out must be **explicit and auditable**, never silent.
- It must not break the modules that already scope correctly: `accounts` derives the
  scope from `&Ctx` and names no `Scope::Global`, so it must pass untouched.

## Decision

Add a **`scope-policy`** fitness function to `seiva lint` (the same tool that
enforces ADR 0003 / ADR 0030), run in CI as a hard gate over `examples/**` and a
project's `modules/**`.

**What it inspects.** For every module, the lint reads the source of its
`application` and `domain` layer crates (the layers that *choose* the scope; the
`domain` layer must never name a scope at all) and flags any **code** use of the
literal `Scope::Global`:

1. **A literal `Scope::Global` in application/domain code is an error by default.**
   This is the audited cross-tenant escape; writing it into a tenant-owning module's
   use case is the bypass BOLA is about. Fail-closed: the unmarked case does not
   pass.
2. **The opt-out is an auditable marker, not a type.** A legitimately cross-tenant
   read (admin, reporting, maintenance, seeding) keeps `Scope::Global` by placing a
   `seiva:allow-global` marker comment on the same line or the line immediately
   above, with a reason — e.g.
   `// seiva:allow-global — cross-tenant reporting read, admin only`. The set of
   marked sites **is** the maintenance allowlist: it is greppable
   (`grep -rn 'seiva:allow-global'`), so a security review can enumerate every
   deliberate cross-tenant access in the codebase from the markers alone.
3. **Comments and doc-strings do not count.** The scan ignores occurrences in `//`
   comments and doc-strings (a `///` reference to `[`Scope::Global`]` is
   documentation, not a bypass) — it looks only at the code before any trailing
   `//`. This is a deliberate heuristic, not a proof (see Consequences).

**Reporting.** The error names the module, the layer, the file and the line, and
states the two ways to resolve it — derive the tenant `Scope` from the `&Ctx`
(the `accounts` pattern) or add a `seiva:allow-global` marker with a reason — the
same ergonomic bar as the layering lint (ADR 0003's diagnostic style).

**Why a literal scan and not the type system.** `Scope::Global` cannot be removed
or made `unsafe`: it is the legitimate escape the gateway audits, and
`Ctx::scope()` itself returns it for an untenanted/anonymous caller. The residual
"anonymous → `Scope::Global`" path in `Ctx::scope()` is **not** closed by this lint
(it lives in `seiva-http`, not a module layer); it is closed at the other end — a
`#[model(tenant_scoped)]` read opened under `Scope::Global` by an untenanted caller
is refused by DORM (`TenantScopeMissing`), so it **fails closed at the gateway**
rather than leaking. The lint closes the *module-author* bypass: a tenant-owning use
case that deliberately writes `Scope::Global` instead of scoping by the caller.

## Consequences

- The largest structural security gap — tenant-scope bypass left to discipline —
  becomes a CI gate. A module that passes a literal `Scope::Global` in its use case
  **fails the build** unless it justifies it with an auditable marker, so the honour
  system is replaced by a fitness function, consistent with how ADR 0003 enforces
  layering.
- `examples/notes` is migrated to the tenant-scoped pattern (derive the `Scope` from
  `&Ctx` server-side, as `accounts` does) and names no `Scope::Global`; `accounts`
  passes unchanged. The single-tenant demos (`polls`, `shop/*`) keep `Scope::Global`
  behind a `seiva:allow-global` marker that records them as deliberate,
  demonstration-only single-tenant code — a visible, tracked exception rather than
  an invisible default, and the backlog item to tenant-scope them is now greppable.
- Security review gains an inventory: every `seiva:allow-global` is a justified,
  reviewable exception. Removing the marker (or omitting it on a new module) turns
  the build red.
- It is a **heuristic, not a proof**. A module could still bypass isolation without
  ever typing `Scope::Global` (e.g. by threading a `Scope` value it computed
  globally). The lint catches the overwhelmingly common, literal form; it is
  strictly additive safety over today's zero checks, and it drives authors toward
  deriving the scope from `&Ctx`, where the common path is safe by construction.

## Alternatives considered

- **Full static data-flow analysis** (track a tenant value from load to response, or
  a `Scope` from construction to `transaction`). Rejected: too expensive and too
  brittle on real Rust; false positives would get the lint disabled. The literal
  scan is cheap, local and runs on information already on the line.
- **A `WireSchema`/facade-descriptor BOLA check** (classify a response DTO as
  tenant-bearing by field name and require it be returned `Scoped<T>`). Explored and
  rejected for now as the *first* gate: it needs the facade descriptor and the
  `WireSchema` of every DTO, its field-name classification is itself a heuristic, and
  it does not catch the actual bypass seen in practice — a use case running under
  `Scope::Global`. The literal scan targets that bypass directly and is far cheaper;
  the descriptor-based check remains a candidate second gate layered on top.
- **Removing `Scope::Global` / making it `unsafe`.** Rejected: it is the legitimate,
  audited cross-tenant escape and the required return for an untenanted
  `Ctx::scope()`. The problem is not its existence but its *unmarked* use in a
  tenant-owning module — which a marker-gated lint addresses without removing the
  escape.
- **A runtime assertion** (panic/`403` if a transaction opens `Scope::Global`).
  Rejected: not fail-closed at compile time, fires only on the executed path (an
  untested path still bypasses), and moves a build-time guarantee into production.
- **Leave it to code review (status quo).** Rejected as the whole reason for the
  ADR: review is exactly the honour system, and BOLA is the number-one API risk — it
  deserves a gate, not a reviewer's memory.
