# Part 2: Validation and DTOs

> **Status: planned.** This page is a planned part of the documentation structure; the outline below is the intended content.

Add a write DTO, validation and RFC 9457 errors.

## On this page

- Define a `CreateRecord` DTO with serde
- `impl Validate` and the `Valid<T>` extractor
- How a validation failure becomes a 422 Problem
- `#[derive(WireSchema)]` so the OpenAPI document shows the real fields

## Related

- [Validation (topic guide)](../../topics/validation.md)
- [Wire schema (ADR 0007)](../../adr/0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../../index.md), organised like the Django
docs (tutorials · topic guides · reference · how-to).*
