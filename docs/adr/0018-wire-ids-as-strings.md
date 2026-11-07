# ADR 0018 — Identifiers cross the wire as JSON strings

- Status: Accepted
- Date: 2026-11-07
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0007

## Context

Seiva identifiers are 128-bit values (`seiva_ports::Id` is a `[u8; 16]`, rendered
as a `u128`/UUID/ULID by adapters). When such an id is exposed on a wire DTO as a
plain `u128`, `serde` serializes it as a JSON **number**.

A JSON number is not a safe carrier for a 128-bit integer. In practice JSON
numbers are IEEE-754 doubles: JavaScript — the single most common API client —
represents every number as a double, so any integer above **2^53**
(`Number.MAX_SAFE_INTEGER`, ~9×10¹⁵) silently loses precision on `JSON.parse`. A
real Seiva id like `1794005853720908638` (19 digits) is well past that line, so a
browser reads a *different* number. Identifiers are the worst place for this: a
mangled id addresses the wrong row, and a round-trip (`GET /v1/catalogs/{id}`
after reading the id from a list) returns `404` or, worse, the wrong record. The
`shop` tutorial's dxjs storefront hit exactly this: navigating to a product by its
id failed because the id had already been truncated at parse time in the browser.

This is a **contract-fidelity** problem in the spirit of ADR 0007: the published
OpenAPI said `{"type":"integer"}` for ids, but no mainstream JS client can honour
that type for the values the server actually emits. The forces:

- The wire value must be **exact** for every client, including JavaScript.
- The framework must not reach for reflection or a bespoke number codec (ADR 0001
  keeps the contract reflection-free and all-compile-time).
- Internal code should keep working with a real integer type, not strings.

## Decision

**Identifiers cross the wire as JSON strings.** The framework provides a
`delonix_seiva::WireId` newtype that wraps a `u128`, serializes as a decimal
**string**, deserializes from a string (and leniently from a number for small
values), and whose `WireSchema` is `{"type":"string","format":"uint128"}`. Wire
DTOs use `WireId` for identifier fields; the domain keeps its native `u128`.

```rust
use delonix_seiva::WireId;

#[derive(Debug, Clone, Serialize, Deserialize, WireSchema)]
pub struct ProductDto { pub id: WireId, pub name: String, pub price_cents: u64 }
// on the wire: {"id":"1794005853720908638","name":"Mug","price_cents":1500}
```

`WireId` converts both ways with `u128` (`From`/`Into`, `get()`, `Display`,
`FromStr`), so the application maps a domain id to the DTO with `.into()` and back
with `.get()` — no hand-written `to_string()`/`parse()` per field. A **path**
parameter may stay a `u128` (axum parses the full digits from the URL segment with
no precision loss); only JSON bodies and responses need `WireId`.

Because `WireId` carries its own `WireSchema`, the OpenAPI document and Swagger UI
describe ids as strings automatically — the document and the wire agree again
(ADR 0007).

## Alternatives considered

- **Leave ids as `u128` (JSON numbers).** The status quo. Rejected: it is silently
  wrong for JavaScript and any IEEE-754 client above 2^53 — the most common
  deployment target — and the failure is a hard-to-trace wrong-record bug, not an
  error.
- **Enable `serde_json`'s `arbitrary_precision`.** Lets the *server* round-trip a
  u128 number, but does nothing for the *client*: the browser still parses it as a
  lossy double. It also changes number handling globally. Rejected: it does not
  solve the actual (client-side) problem.
- **A per-field `#[serde(with = "…")]` string helper.** Works, but leaves the
  `WireSchema` saying `integer` unless each field *also* carries a schema override
  — two things to remember per id, easy to get half-right (a string on the wire
  documented as an integer). Rejected in favour of one type that carries both the
  serialization and the schema.
- **Always use a UUID/ULID string type.** A larger change to the id model
  (`seiva_ports::Id`) and to adapters. Out of scope here; `WireId` is about how an
  id is *carried*, not what it *is*, and composes with a future UUID rendering.

## Consequences

- **Easier:** any client — JavaScript first — gets exact ids for free; the OpenAPI
  type matches the wire; DTOs read naturally (`id: WireId`); mapping is `.into()` /
  `.get()`.
- **Harder / debt:** id fields are now strings in the JSON, so a client that was
  (incorrectly) treating them as numbers must change — a one-time, correctness-
  positive break. The `shop` example adopts `WireId` as the reference; other
  examples that expose ids should migrate as they are touched (accepted debt,
  2026-11-07).
- **Enforcement:** `WireId` lives in `delonix-seiva` with unit tests (string
  serialization, large-value round-trip, string-or-number deserialization, schema
  is `string`). The convention is documentary — new DTOs choose `WireId` for ids;
  reviewers reject a bare `u128`/`i64` id on a wire DTO, pointing here.

## Related

- [ADR 0001](0001-seiva-foundations.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md)
- The reference adoption: [`examples/shop`](https://github.com/ngolacloud/seiva/tree/main/examples/shop)
  and the separate dxjs storefront that consumes it.

---

*Part of the [Seiva documentation](../index.md).*
