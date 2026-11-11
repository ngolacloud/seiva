# ADR 0025 — WireSchema reflects serde, or fails to compile

- Status: Accepted
- Date: 2026-11-10
- Deciders: NgolaCloud
- Follows: ADR 0007

## Context

`#[derive(WireSchema)]` (ADR 0007) lets a DTO describe its own JSON shape at
compile time, with no reflection, so `/openapi.json`, the generated `/admin`
console and the gRPC proto all document the real wire. The derive read the struct
fields and emitted their **Rust** names and an `Option`-based `required`.

But the bytes on the wire are produced by `serde`, and the derive **ignored every
serde attribute**: `#[serde(rename_all = "camelCase")]`, `#[serde(rename = "…")]`,
`skip`, `skip_serializing_if`, `default` and `flatten` all move the JSON, yet none
changed the published schema. A DTO that paired `serde` with `WireSchema` therefore
published a schema that **silently disagreed with the JSON `axum::Json` serialises**
— the OpenAPI/admin/proto lied, with no compile-time warning. The engagement
flagged this as the only remaining Critical (SC-1): latent only because no example
in the repo combined the two, but the first step of a real user.

A value-reshaping attribute (`#[serde(with = "…")]`, `serialize_with`,
`deserialize_with`) is worse still: it changes the *type* on the wire (e.g. bytes
as a base64 string) to something the schema cannot infer from the Rust type.

## Decision

**The derive reflects the serde attributes that move the wire, and fails to
compile on the ones it cannot reflect faithfully.** It is reflect-or-reject —
never a silent lie.

Reflected (schema now matches the serialised JSON, proven by tests that compare
the schema's property names to `serde_json::to_value(instance)`):

- `#[serde(rename_all = "…")]` (container) — all eight serde case rules, applied
  with serde's own field/variant conventions;
- `#[serde(rename = "…")]` (field and enum variant) — the explicit wire name;
- `#[serde(skip)]` / `skip_serializing` — the field leaves the schema entirely;
- `#[serde(skip_serializing_if = "…")]`, `#[serde(default)]` (field or container),
  `skip_deserializing` — the field is **not** `required`;
- `#[serde(flatten)]` — the nested object's `properties`/`required` merge into the
  parent, as serde flattens them onto the wire.

Rejected at compile time (would make the schema lie, so better an actionable
error): `#[serde(with/serialize_with/deserialize_with)]` on a field and
`#[serde(transparent)]` on a container. An unrecognised `rename_all` rule is also
a compile error rather than an unchecked name.

Unmodelled serde keys that do **not** move the JSON object shape
(`deny_unknown_fields`, `bound`, `crate`, enum tagging, …) are ignored.

## Consequences

- OpenAPI, the admin console and the gRPC proto now describe the real wire for a
  DTO that uses serde attributes — the Critical SC-1 is closed.
- A DTO that reaches for an unsupported, wire-reshaping serde attribute no longer
  builds, pointing the author at the attribute instead of shipping a wrong schema.
- `schema::Field` gains a `flatten: bool`; `object_schema` merges a flattened
  field's properties and required entries into the parent.
- Full value-level `oneOf` for data-carrying enums and base64/`with` shapes remain
  future work; today they are rejected, not mis-described.
