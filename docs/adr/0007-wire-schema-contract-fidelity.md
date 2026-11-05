# ADR 0007 — Wire-schema contract fidelity: DTOs describe their own shape

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0003

## Context

A facade descriptor (`#[seiva::facade]`, ADR 0001) carries each method's
parameter and return **types as Rust type-name strings** (`"CreateNote"`,
`"Result<NoteDto, String>"`), never a structural description — the "zero
reflection" rule. The OpenAPI generator (`seiva-openapi`) therefore produced an
**unfaithful** contract in two ways:

1. **The `cmd` envelope.** A write method's request body was documented as an
   `object` whose properties were the parameters keyed by name
   (`{"properties":{"cmd":…},"required":["cmd"]}`). But the generated
   `#[seiva::controller]` deserialises the body **as the parameter's DTO
   directly** (`seiva_http::Valid<CreateNote>`) and rejects more than one body
   parameter. A client following the document would send `{"cmd":{…}}` and the
   server would reject it — the documented and the accepted shapes diverged.

2. **The `x-seiva-rust-type` placeholder.** For any non-scalar type the body and
   response schemas were `{"type":"string","x-seiva-rust-type":"CreateNote"}` —
   claiming a *string* where the wire carries a JSON *object* with named fields.

Together these are the "contract fidelity" blocker: the generated OpenAPI was not
a contract a client could code against. The forces: the generator cannot see
inside a named type without either reflection (rejected by ADR 0001) or the type
describing itself; and whatever we add must not break the descriptor shape the
gRPC generator and the `[[uses]]`/`[[allow]]` checks already read.

## Decision

**A DTO contributes its own JSON Schema through a `WireSchema` trait, derived at
compile time and resolved by the generator through a link-time inventory — the
same mechanism `#[seiva::facade]` already uses to discover facades. The OpenAPI
generator documents a write body as the parameter DTO directly, not an
envelope.**

1. **`delonix_seiva::schema::WireSchema` (new).** `fn schema() -> serde_json::Value`
   returns a JSON Schema fragment. Implemented here for the well-known scalars,
   `Vec<T>`, `Option<T>` and `&T` (so field types compose), mirroring
   `seiva_openapi::schema_for_rust_type` so a field's schema is identical whether
   reached by name or by composition.

2. **`#[derive(WireSchema)]` (new, in `seiva-macros`, re-exported as
   `delonix_seiva::WireSchema`).** For a struct with named fields it implements
   `WireSchema` (an `object` schema whose properties compose each field's
   `<FieldTy as WireSchema>::schema()`; an `Option<_>` field is omitted from
   `required`) **and** emits one `inventory::submit!` registering the type's name
   → schema. It rejects tuple/unit structs, enums and generics, which have no
   name→shape mapping to register.

3. **The generator resolves by name, additively.** `schema_for_rust_type` looks a
   named type up via `delonix_seiva::schema::registered_schema` before falling
   back to the placeholder. A type that did **not** derive `WireSchema` is absent
   from the inventory and still documents honestly as a placeholder. **No
   descriptor field changed** — `Param`/`MethodDescriptor` keep `ty: &'static str`
   — so the gRPC generator and the contract checks are untouched.

4. **The body is the DTO.** For a `POST` with exactly one wire (non-`&Ctx`) body
   parameter, the request-body schema is that parameter's schema directly; with
   none, there is no body. The old keyed-object remains only as a fallback for a
   hand-built descriptor declaring more than one body parameter (unreachable
   through `#[controller]`).

The example DTOs (`notes`, `accounts`) and the scaffolded module template derive
`WireSchema`, so every facade the framework ships and generates documents its
real shape.

## Alternatives considered

- **Thread a schema provider into the descriptor** (`Param.schema: fn() -> Value`,
  emitted by `#[facade]`). Rejected: it changes the `const DESCRIPTOR` shape read
  by the gRPC generator and the contract checks, forces a `WireSchema` bound on
  *every* parameter/return type (including `&Ctx`), and complicates the
  hand-written equivalent — a far larger blast radius than the by-name inventory,
  for the same result.
- **A meta-schema/reflection crate (e.g. `schemars`).** Rejected by ADR 0001's
  zero-reflection rule and the DORM-free/thin-dependency posture; `WireSchema` is
  a derive, resolved at compile time.
- **Leave the placeholder, only fix the envelope.** Rejected: removing the
  envelope leaves a bare `type:string` placeholder claiming the whole body is a
  string — more misleading than before. The two defects are coupled.

## Consequences

- The generated OpenAPI is now a faithful contract: `seiva openapi` documents the
  `notes` create body as `{title}` and the response as `{id,title}`, with zero
  `x-seiva-rust-type` placeholders for `WireSchema` DTOs.
- A facade DTO must derive `WireSchema` to be documented structurally; one that
  does not degrades gracefully to the named placeholder (no breakage).
- Resolution is link-time, like facade discovery: a standalone tool sees only the
  DTOs compiled into it; the runtime binary that mounts the modules produces the
  complete document. Two DTOs sharing a short name across crates collide (first
  match wins) — a pre-existing property of name-keyed descriptors.
- gRPC/proto fidelity is **unchanged** (it reads `ty` and keeps its own mapping);
  enriching the proto message from `WireSchema` is a possible follow-up.
- `Vec<u8>` composes as an array of integers rather than the string mapper's
  `format:"byte"` special case — a minor, documented divergence for composed
  field schemas.

## Addendum (2026-11-05) — enums, bytes and error responses

Extending the fidelity work (§8) without changing the decision above:

- **Unit-variant enums.** `#[derive(WireSchema)]` now accepts a C-like enum and
  emits `{"type":"string","enum":[…variant names…]}` (`schema::enum_schema`),
  matching serde's default serialization of a unit variant as its name. A
  **data-carrying** variant is a clear compile error (a faithful `oneOf` is future
  work), and serde `rename`/`rename_all` is not reflected — name wire-enum
  variants as they serialize. Nested enums inline into their containing object.
- **Bytes.** `Vec<u8>` composes as an array of integers — faithful to
  `serde_json`'s default wire form (a base64 `format:"byte"` string would need a
  serde attribute), now covered by a test.
- **Error responses.** The OpenAPI generator documents error responses as
  `application/problem+json` referencing a shared `Problem` schema in
  `components.schemas`: a guarded operation documents `401`/`403`, and any
  operation with a request body documents `422`. `components` is therefore always
  present (it carries `Problem`); the bearer scheme is still added only when a
  method is guarded.
- **gRPC** remains proto-generation only; a served round-trip is explicitly not
  claimed, so none is asserted.
