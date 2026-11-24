# ADR 0030 — The data layer is pluggable: DORM is the default adapter, not a requirement; sqlx and SeaORM are first-class

- Status: Accepted
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0002 (core ports and in-memory doubles), ADR 0004 (the DORM data-layer gateway), ADR 0011 (idempotency as a port, reusing DORM), ADR 0017 (pluggable password hashers + a reference flow)

## Context

The boundary is already neutral. `seiva-ports` carries no data-layer dependency
and no signature leaks an ORM type: `UnitOfWork`, `EventPublisher`, `Clock`,
`IdGenerator`, `IdempotencyStore`, `AuditSink` and `RateLimitStore` are stated in
terms of `Scope`, `Id`, `PortResult` and primitives, never `Session`,
`QueryBuilder` or SQL. Each module defines its own repository port in its
`domain`, in terms of its own entities (`examples/notes/domain` → `NoteRepository`).
Two independent implementations of these ports already exist — the DORM gateway
(`seiva-data-dorm`) and the in-memory doubles (`seiva-testing`) — which is the
living proof that the contract is not welded to DORM. The application and domain
layers of every `examples/` module import no ORM at all; the dependency rule is
enforced by the crate graph and `seiva lint` (ADR 0003).

So the architecture is right. The **product story is not.** Three things make
DORM read as the *only* realistic path rather than the *default* one:

- **There is only one reference adapter.** `seiva-data-dorm` is the single crate
  that implements the ports end to end. A reader who wants sqlx or SeaORM finds a
  hole and a promise, not a worked example.
- **The CLI hardcodes DORM.** `seiva db init/dev/migrate/pull/studio` shell out to
  the `dorm` binary (`seiva-cli`), and `seiva module new --data external` answers
  *"not implemented yet"*. A non-DORM user gets no scaffold and no migration path
  from the framework's own tooling.
- **The README frames the alternative as a void.** It says to *switch DORM off and
  bring your own connections* — a do-it-yourself vacuum, not a supported adapter.

This matters because it is the single largest **adoption barrier** for the
framework. Asking a Rust developer to take on Seiva **and** a proprietary ORM at
once doubles the perceived risk, next to a sqlx or SeaORM they already run in
production. The Rust community reaches for stacks it already trusts; a full-stack
framework that *requires* its own ORM self-selects a tiny audience. The competitors
confirm the gap: Loco weds SeaORM, cot weds its own — neither lets you bring the
data layer you already have.

The forces:

- **DORM stays the default, and must stay worth defaulting to.** The batteries —
  migrations, the transactional outbox (ADR 0011), the admin studio, per-module
  schema — are what separate Seiva from "axum + sqlx wired by hand". A project that
  starts from zero should get the shortest path, and that path is DORM.
- **The framework must not impose an ORM.** The neutrality the ports already have
  must become a *product promise*, not an architectural accident one refactor away
  from rotting.
- **The core stays DORM-free and ORM-free.** Like the rate limit (ADR 0015) and the
  password hasher (ADR 0017), the mechanism lives in the framework but the concrete
  backend is chosen at the composition root, behind a cargo feature, linking nothing
  when unused.
- **Neutrality must be enforced, not trusted.** If `application`/`domain` can name
  an ORM crate and CI stays green, the boundary erodes the first time someone is in
  a hurry.

## Decision

**The Seiva data layer is a set of ports (`seiva-ports`); DORM is the default
reference adapter, and sqlx and SeaORM are first-class reference adapters
maintained beside it. A project picks its adapter at the composition root; the
core, the application and the domain never name an ORM.**

**The contract.** `seiva-ports` is the whole data-layer surface: the service ports
(`UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator`, `IdempotencyStore`,
`AuditSink`, `RateLimitStore`) plus the per-module repository port each module
declares in its `domain` in terms of its own entities. No ORM type crosses this
boundary — a verifiable invariant that already holds today.

**Reference adapters.** Each is a `seiva-data-<x>` crate behind its own cargo
feature, none in the default workspace `members`, none linked by a core crate:

- `seiva-data-dorm` (**default**, feature `dorm`): migrations, the transactional
  outbox, studio, admin — the batteries-included experience. Unchanged by this ADR.
- `seiva-data-sqlx` (feature `sqlx`): the ports over `sqlx::Transaction`; the user
  brings their own schema and migrates with `sqlx-cli`. This is the proof that
  Seiva is agnostic *even of an ORM* — it runs on a bare query layer.
- `seiva-data-seaorm` (feature `seaorm`): the ports over SeaORM's
  `db.transaction()`; the user migrates with `sea-orm-cli`. This is the direct
  answer to Loco — the same ORM, under a framework that does not force it on you.

**The transaction contract is "ambient".** `UnitOfWork::transaction(scope, closure)`
is scope-based, not handle-based: the closure runs inside the transaction, commit
on `Ok`, rollback on `Err`, and no transaction handle is passed to the body. Each
adapter establishes the current transaction in its own task-local so the module's
repositories find it — DORM via `db::atomic`, sqlx via `begin`/`commit`, SeaORM via
`transaction()`. This "ambient transaction" pattern **is** the contract every
adapter honours; it is documented as such, not re-invented per backend.

**Default and developer experience.** DORM remains the scaffold default
(`seiva module new --data dorm`); `--data sqlx | seaorm | external` selects another
adapter. The DORM-specific tooling (`seiva db migrate/pull/studio`) keeps delegating
to DORM when the adapter *is* DORM; for sqlx/SeaORM the CLI points at the native tool
(`sqlx migrate`, `sea-orm-cli`) instead of pretending. The CLI mechanics are a
separate slice; this ADR fixes the **policy**, not the plumbing.

**Neutrality becomes a fitness function.** `seiva lint` gains a check that fails if
an `application`, `domain` or `facade` crate names a data-adapter crate
(`seiva-data-*`) or an ORM type. The pure layers code against the ports; the `facade`
is covered too as the published contract (the single source of OpenAPI + proto +
routes) — defence-in-depth over the `WireSchema` boundary, which already keeps the
wire DTOs ORM-free. The one-way, ORM-free boundary stops being a convention and
becomes a CI gate; CI builds all three reference adapters against the ports.

## Alternatives considered

- **Keep DORM as the only realistic path (status quo).** Rejected as the whole
  point: it is the framework's largest adoption barrier, turns away everyone with
  sqlx/SeaORM already in production, and squanders the fact that the contract is
  *already* neutral. A neutral boundary nobody can see is worth nothing.
- **Abstract a universal "Seiva ORM" that wraps any backend** (a query builder over
  the top). Rejected: it would re-implement sqlx/SeaORM/Diesel badly, pin every user
  to a lowest-common-denominator query language, and contradict "do not impose an
  ORM". Seiva abstracts the **service ports** (transaction, idempotency, outbox,
  audit), not the query language — each adapter speaks its own.
- **Make sqlx or SeaORM the default instead of DORM.** Rejected: DORM is what
  delivers the batteries-included experience (migrations, admin, studio, outbox all
  wired) that distinguishes Seiva from hand-assembled axum, and it is the family's
  product. The answer is *default DORM, open to all*, not a weaker default.
- **Let the user write their own adapter with no reference (the "switch it off and
  bring your own" status quo).** Rejected: a promise without a worked adapter does
  not convert — the community has to *see* sqlx and SeaORM compiling against the
  ports to believe it. The precedent is ADR 0017: pluggable password hashers shipped
  *with* a reference credential flow, not as a bare trait.

## Consequences

- **The positioning flips.** Seiva becomes "the full-stack Rust framework that does
  not wed you to an ORM" — a differentiator neither Loco (SeaORM) nor cot (its own)
  can claim. Adoption barrier #2 (a proprietary ORM) turns into the headline feature.
- **There is a migration path in.** A team already on sqlx or SeaORM adopts Seiva
  without rewriting persistence: implement the ports, keep the existing schema and
  queries. The cost of trying the framework drops from "learn a new ORM too" to "wire
  an adapter".
- **DORM keeps winning the default.** It is the richest experience, not the only one;
  the from-zero developer still gets the shortest path, and the family's ORM is what
  the quickstart shows.
- **Debt we accept (2026-11-15):** three reference adapters to maintain — the CI
  matrix grows, and every new port must land in all three. Mitigated by the port set
  being small and stable (ADR 0002) and by the sqlx/SeaORM adapters being lean
  reference examples, not products; if a port cannot be met by a plain query layer,
  that is a signal the port is wrong, caught early by the sqlx adapter.
- **Enforcement, not trust:** neutrality is checked by `seiva lint` and the boundary
  is a CI gate, so it cannot silently rot.
- **Out of this ADR:** the DORM-specific migration CLI for non-DORM adapters
  (`seiva db migrate/pull/studio`) is a separate slice; until it lands, sqlx/SeaORM
  users run their own migration tool outside `seiva db`. (`module new --data
  sqlx|seaorm|external` itself did land under this ADR's adapter slice, scaffolding
  per adapter and pointing at the native tool.) This ADR commits the policy that
  makes the remaining slice worth doing.
