# ADR 0019 — Authorization is fail-closed: every facade method declares its policy

- Status: Accepted
- Date: 2026-11-08
- Deciders: NgolaCloud
- Follows: ADR 0005, ADR 0006

## Context

A facade method states the permission a caller needs with
`#[requires("perm")]` (ADR 0006); `#[seiva::controller]` mounts the
`permission_guard` from the method's `MethodDescriptor.requires`, and the guard
treats an **empty** requirement as "no authorization" and lets the request
through.

Until now the empty requirement was also the *default*: a method with no
`#[requires(..)]` attribute compiled to `requires: ""` and so to a **public
route**. The failure direction was the dangerous one — *forgetting* to guard a
method did not fail the build, it silently exposed the route to anonymous callers
(OWASP API5, Broken Function-Level Authorization). Nothing in the type system or
the build distinguished "deliberately public" from "author forgot". A security
review of the framework flagged this as the one default that could compromise any
service built on Seiva without the author noticing.

The forces:

- A missing access policy must be **impossible to ship**, not a silent default.
- A genuinely public endpoint (a health echo, a public catalogue read) must stay
  expressible — fail-closed must not mean "everything is guarded".
- The decision must be **auditable**: a reviewer should see, at the method, that
  public exposure was chosen on purpose.
- No change to the wire, the routes, or the `MethodDescriptor` shape (ADR 0007):
  an empty `requires` is already the guard's public sentinel.

## Decision

**Every facade method must declare its access policy at compile time.** A method
with neither `#[requires("perm")]` nor the new `#[public]` attribute is a
**compile error**, with an actionable message:

```text
facade method `delete` declares no access policy: add `#[requires("permission")]`
to guard it, or `#[public]` to expose it without authorization (an explicit,
auditable choice)
```

`#[public]` is the explicit, auditable opt-out of the permission guard. It takes
no arguments and keeps `requires` empty — the same sentinel the guard already
reads as "no authorization" — so the `MethodDescriptor`, the generated route and
the OpenAPI `security` block are byte-for-byte what a hand-written empty
`requires` produced before. Only the *default* changed: absence of a policy is now
an error, not a public route.

```rust
#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, ctx: &Ctx, cmd: CreateNote) -> Result<NoteDto, String>;

    #[public] // deliberately unauthenticated — an audited choice, not a default
    async fn ping(&self) -> Result<(), String>;
}
```

This is the authorization half of a broader fail-closed hardening pass; the other
changes (body-limit-honouring `Valid<T>`, unknown-field rejection, non-leaking
`500`s, tenant-scoped rate limiting, SHA-256-indexed tokens, boot-time route-
collision detection) are internal and need no contract decision.

## Alternatives considered

- **Keep the silent public default.** The status quo. Rejected: the failure mode
  (forget `#[requires]` → anonymous route) is exactly backwards for a security
  control, and nothing surfaces it.
- **Deny at runtime instead (guard rejects an empty requirement).** Would make a
  forgotten policy a `403` rather than a public route, but also breaks every
  genuinely public endpoint unless re-marked, and the failure shows up only when
  the route is hit, not at build. Rejected: compile-time is earlier and total.
- **A crate-level lint (`seiva lint`) that flags un-annotated methods.** Catches
  the same mistake, but only when the lint is run, and it is advisory. Rejected in
  favour of a hard compile error that no pipeline can skip.
- **A `#[requires]` with an empty string as the public marker.** Already how the
  guard reads public, but `#[requires("")]` reads like a typo and is easy to write
  by accident — the opposite of auditable. `#[public]` names the intent.

## Consequences

- **Easier:** a forgotten access policy can no longer ship — it fails the build at
  the method with a message that names the fix. Public exposure is visible at the
  call site (`#[public]`), so a reviewer audits it directly.
- **Harder / debt:** an existing facade method that relied on the silent public
  default must now say `#[public]` explicitly. This is a one-time, security-
  positive break; the in-tree examples and the controller test were migrated with
  this ADR (every example facade already declared `#[requires]`, so only the
  deliberately-public `list`/`ping` methods needed the marker).
- **Enforcement:** the gate lives in `#[seiva::facade]` (`seiva-macros`); the
  `delonix-seiva` facade test and `seiva-controller-test` exercise both a guarded
  and a `#[public]` method, so a regression that re-opens the silent default fails
  the build.

## Related

- [ADR 0005](0005-object-level-authorization-bola.md) · [ADR 0006](0006-identity-and-authentication.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../index.md).*
