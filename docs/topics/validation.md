# Validation and DTOs

> **Status: planned.** This page is a planned part of the documentation structure; the outline below is the intended content.

Validated input, honest error responses.

## On this page

- `serde` DTOs as the wire types
- `impl Validate` and the `Valid<T>` extractor
- `ValidationError` → RFC 9457 `422`
- `#[derive(WireSchema)]` publishes the real fields to OpenAPI

## Related

- [Wire schema (ADR 0007)](../adr/0007-wire-schema-contract-fidelity.md)
- [API documents](api-documents.md)

---

*Part of the [Seiva documentation](../index.md), organised like the Django
docs (tutorials · topic guides · reference · how-to).*
