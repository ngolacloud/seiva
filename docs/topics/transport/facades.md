# Facades: the contract

A **facade** is a module's versioned public contract: a trait annotated with
`#[seiva::facade]`. It is the single source of truth — the HTTP routes, the
OpenAPI document, the proto service and the permission checks are all derived from
it, never hand-written.

## Declaring a facade

```rust
use delonix_seiva::{facade, WireSchema};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, WireSchema)]
pub struct CreateNote { pub title: String }

#[derive(Debug, Clone, serde::Serialize, WireSchema)]
pub struct NoteDto { pub id: u128, pub title: String }

#[facade(version = "v1")]
pub trait Notes {
    #[requires("notes.create")]
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String>;
}
```

- **`#[facade(version = "v1")]`** — marks the trait as a contract at a version.
  Versions live side by side (`/v1/…`, `/v2/…`).
- **`#[requires("notes.create")]`** — the permission a caller must hold for this
  method. Read from the descriptor and enforced before the handler runs.
- **DTOs** are the wire types: they derive `serde` and
  [`WireSchema`](../../reference/wire-schema.md) so the OpenAPI document carries
  their real fields (not an opaque placeholder).

## What it generates (no hidden magic)

The macro only *adds* a zero-sized `NotesFacade` whose `DESCRIPTOR` lists the
name, version and methods (each with its `requires` and signature), and registers
it in a link-time inventory. The hand-written equivalent is documented in
`delonix_seiva::contract`; run `seiva expand` to see it. Nothing is reflective —
the descriptor is known at compile time.

Because every facade registers itself, a running service's `/openapi.json` (and
`seiva openapi`/`proto`) lists **every mounted module** with no central registry
to maintain — closing OWASP API9 (improper inventory).

## The layers around it

The facade is the top of a module's five layers: `domain` (entities/rules),
`application` (the use case over ports), `infrastructure` (adapters), and
`interface` (the controller). The facade's DTOs and trait are what the other
modules and the transports see; the implementation stays private. See
[Architecture](../../architecture.md).

## Related

- [Controllers and routing](controllers-and-routing.md) · [Validation](../validation.md)
- [The contract descriptor](../../reference/contract-descriptor.md) · [ADR 0007 — WireSchema](../../adr/0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../../index.md).*
