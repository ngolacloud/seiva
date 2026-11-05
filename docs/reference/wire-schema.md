# Wire schema

`WireSchema` lets a DTO describe its own JSON shape so the OpenAPI document carries
the DTO's **real fields**, not an opaque placeholder — with no reflection (ADR
0007).

## The trait (`delonix_seiva::schema`)

```rust
pub trait WireSchema { fn schema() -> serde_json::Value; }
```

Implemented in the framework for the well-known types — `String`, `str`, `bool`,
the integers (as `{"type":"integer","format":"<ty>"}`), `f32`/`f64`, `()`,
`Vec<T>`, `Option<T>`, and `&T` (which forwards) — so field types compose.

## `#[derive(WireSchema)]`

On a struct with named fields it:

1. implements `WireSchema::schema()` as an `object` schema whose properties are the
   fields, each field's schema composed from its type (`<FieldTy as WireSchema>::schema()`);
   an `Option<_>` field is left out of `required`;
2. registers the type by name in a link-time inventory
   (`SchemaRegistration { name, schema }`).

It rejects tuple/unit structs, enums and generics (no name→shape mapping).

## Resolution

A generator holding a type-name string resolves the real schema via
`delonix_seiva::schema::registered_schema(name)`. `seiva-openapi`'s
`schema_for_rust_type` checks the scalars first, then the inventory, and only then
falls back to the honest placeholder `{"type":"string","x-seiva-rust-type":"<ty>"}`
for a named type that did not derive `WireSchema`.

## Why it is additive

The facade descriptor still carries only `ty: &str`; `WireSchema` registers shapes
separately. So adding it changed no descriptor and left the gRPC generator and the
contract checks untouched — a DTO opts in by deriving it, and one that does not
degrades gracefully to the placeholder.

## Related

- [API documents](../topics/api-documents.md) · [Validation](../topics/validation.md)
- [The contract descriptor](contract-descriptor.md) · [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../index.md).*
