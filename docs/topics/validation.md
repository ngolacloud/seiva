# Validation and DTOs

> Seiva's equivalent of Django's **forms** — validated input at the boundary, with
> honest, machine-readable errors.

A facade method's wire types are its own **DTOs** (never domain entities). A write
DTO declares its shape with `serde`, its rules with `Validate`, and its schema with
`WireSchema`.

```rust
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateRecord { pub title: String }

impl Validate for CreateRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}
```

## The `Valid<T>` extractor

The generated controller reads a write body with `seiva_http::Valid<T>`: it
deserialises the JSON into `T` and then calls `T::validate()`. The body **is** the
DTO — there is no wrapper object. On failure:

- a malformed JSON body or a type mismatch → `400`;
- a `ValidationError` from `validate()` → **`422 Unprocessable Entity`** as an RFC
  9457 `Problem` carrying your message.

This is stricter than FastAPI's default: validation is a method on the type, run at
the boundary, and the failure is a typed problem response.

## Where validation lives

Keep *input* validation in `Validate` (shape, required fields, formats). Keep
*business rules* in the domain layer (e.g. a `Record::new` that rejects an empty
title) — the domain enforces invariants regardless of transport. The DTO validates
the request; the domain validates the truth.

## Schema fidelity

Because the DTO derives [`WireSchema`](../reference/wire-schema.md), the OpenAPI
document shows its real fields and which are required — so a client sees exactly
what `Valid<T>` will accept, with no drift between the docs and the server (ADR
0007).

## Related

- [Controllers and routing](transport/controllers-and-routing.md) · [Requests and responses](transport/request-and-response.md)
- [API documents](api-documents.md) · [ADR 0007 — WireSchema](../adr/0007-wire-schema-contract-fidelity.md)

---

*Part of the [Seiva documentation](../index.md).*
