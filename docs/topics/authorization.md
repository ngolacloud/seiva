# Object-level authorization

Authentication says *who* the caller is (a verified `Principal`); authorization
says *what they may do*. Seiva has two layers, and pushes the hard one to compile
time.

## Function-level: `#[requires]`

Each facade method declares the permission a caller must hold:

```rust
#[requires("notes.create")]
async fn create(&self, cmd: CreateRecord) -> Result<RecordDto, String>;
```

`seiva_http::permission_guard` reads that from the descriptor at run time and
enforces it before the handler runs: a caller without `notes.create` gets `403`.
This is OWASP API5 (broken function-level authorization), closed by construction —
the permission travels with the contract, not a hand-written middleware list.

## Object-level: `Scoped<T>` (BOLA)

Function-level permission is not enough: a caller allowed to read *a* note must
still not read *someone else's* note. That is OWASP API1 (Broken Object Level
Authorization), the most common API breach. Seiva's `Scoped<T>` makes it a
**compile-time, fail-closed** concern (ADR 0005): a value that crosses the
boundary is wrapped so that reading it *requires* proving the caller's scope
(tenant/owner) matches. Code that forgets the check does not compile, rather than
silently serving another tenant's data.

```rust
// A Scoped<Note> cannot be unwrapped without a Principal whose scope it matches;
// an out-of-scope access is an error, never a silent leak.
let note: Note = scoped.for_principal(&principal)?;
```

## Tenant isolation, end to end

The tenant boundary runs through the stack: the data layer forbids relations
across module `namespace`s (DORM-B2), and `Scoped<T>` enforces the per-row owner
at the service boundary. Together they make cross-tenant access a type error, not
a code-review hope.

## Related

- [Authentication](authentication.md) · [Security](security.md)
- [ADR 0005 — BOLA](../adr/0005-object-level-authorization-bola.md) · [ADR 0006 — Identity](../adr/0006-identity-and-authentication.md)

---

*Part of the [Seiva documentation](../index.md).*
