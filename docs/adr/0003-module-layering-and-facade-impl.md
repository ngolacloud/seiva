# ADR 0003 — Module layering and where the facade is implemented

- Status: Accepted
- Date: 2026-11-03
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0002

## Context

A module has five layers, one per crate, with the dependency rule enforced by
the crate graph (brief, Parte A). The brief fixes what each layer may depend on
but leaves one detail to "the core ADR": the facade trait is the module's
**inbound** contract (`#[seiva::facade]`), yet it must be implemented somewhere,
and that implementation has to map a use case's result (a domain entity) to the
facade's DTO. Which layer owns that implementation decides the whole wiring.

The constraint that forces the answer: `interface` must not depend on `domain`
(brief's table), so `interface` cannot map a domain entity to a DTO. And
`facade` may depend only on `shared/kernel` and the Seiva macros, so it cannot
implement itself. The brief's table, however, explicitly allows `application` to
depend on **"DTOs da própria facade"** — its own facade crate.

## Decision

1. **The facade trait is implemented in `application`.** The application service
   (e.g. `NotesService`) implements the module's `#[seiva::facade]` trait,
   running the use case and mapping the domain result to the facade DTO. This is
   the hexagonal "inbound port implemented by the application service". It is
   allowed because `application` may depend on its own `facade` crate for the
   DTOs and the trait.

2. **The layer rule is about *direct* dependencies.** `application` depends
   directly on its `facade` crate, which in turn depends on `delonix-seiva` (the
   facade macro). That transitive link does not violate "application must not
   depend on `delonix-seiva`": the rule forbids *direct* coupling to the
   framework runtime, and `application` names no `delonix_seiva::` item. The
   `seiva lint` layer rule (a later fitness function) checks direct dependencies.

3. **`interface` depends on `application` + `facade` only.** It holds a value of
   the facade trait (`S: Notes`) and exposes it over transport (HTTP/gRPC, a
   later phase). It never sees `domain`.

4. **The outbound port (`NoteRepository`) lives in `application`** in the
   domain's language, and is implemented in `infrastructure`. It is a
   module-specific port, distinct from the framework's cross-cutting ports in
   `seiva-ports` (`UnitOfWork`, `EventPublisher`, `Clock`, `IdGenerator`), which
   the application also consumes.

## Consequences

- The dependency graph of a module is: `facade → delonix-seiva`; `domain →`
  (nothing / `shared/kernel`); `application → domain, facade, seiva-ports`;
  `infrastructure → application, domain, seiva-ports`; `interface →
  application, facade`.
- A use case is tested by constructing the application service with the
  `seiva-testing` doubles plus a fake repository — no server, broker or
  database (ADR 0002), proven by `examples/notes`.
- The error boundary: inside the application, ports return `PortResult`
  (`BoxError`); at the facade trait the error is mapped to the contract's error
  type, so internal detail does not leak across the module boundary.

## Alternatives considered

- **Facade implemented in `interface`.** Rejected: `interface` would need to map
  a domain entity to a DTO, which requires depending on `domain` — forbidden.
- **A dedicated adapter crate for the facade impl.** Rejected as unnecessary: it
  adds a sixth crate for what is naturally the application service's job.
- **Use case returns the DTO directly (no domain entity).** Rejected: the use
  case works in the domain's language; mapping to the DTO is a boundary concern
  owned by the application service, not a reason to leak DTOs into the core.
