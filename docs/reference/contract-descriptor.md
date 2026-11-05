# The contract descriptor

`#[seiva::facade]` turns a trait into a machine-readable **descriptor** — the
value every generator (OpenAPI, proto) and check (permissions, inventory) reads.
It is known at compile time; there is no reflection.

## The types (`delonix_seiva::contract`)

```rust
pub trait Facade { const DESCRIPTOR: FacadeDescriptor; }

pub struct FacadeDescriptor {
    pub name: &'static str,         // the trait identifier, e.g. "Notes"
    pub version: &'static str,      // e.g. "v1"
    pub methods: &'static [MethodDescriptor],
}

pub struct MethodDescriptor {
    pub name: &'static str,         // the method name
    pub requires: &'static str,     // the #[requires("…")] permission, or ""
    pub params: &'static [Param],   // the parameters (no &self)
    pub output: &'static str,       // the return type, rendered canonically
}

pub struct Param { pub name: &'static str, pub ty: &'static str }
```

`MethodDescriptor::is_guarded()` is `!requires.is_empty()`.

## Link-time discovery

`#[seiva::facade]` also emits one `inventory::submit!` of a `FacadeRegistration`,
so every facade linked into a binary is discoverable through
`registered_facades()` — sorted by `(name, version)` — with no hand-maintained
list. This is how a running service's `/openapi.json` lists every mounted module
(OWASP API9), and how `seiva openapi`/`proto` document the project.

## No hidden magic

The macro only *adds* the descriptor; writing it by hand is the exact equivalent
(documented in the `contract` module). Run `seiva expand` to see the generated
code for any facade.

## How consumers read it

- `seiva-openapi` turns descriptors into an OpenAPI 3.1 document — resolving each
  named type's real shape from the [wire-schema inventory](wire-schema.md) when
  the DTO derives `WireSchema`, falling back to an honest placeholder otherwise.
- `seiva-grpc` turns them into a proto3 service.
- `#[seiva::controller]` reads the same descriptor to mount routes and enforce
  each method's `requires`.

## Related

- [Facades (topic guide)](../topics/transport/facades.md) · [Wire schema](wire-schema.md)
- [The contract and the macros (internals)](../internals/the-contract-and-macros.md)

---

*Part of the [Seiva documentation](../index.md).*
