# Serialization

Seiva serialises on the wire with **serde**. A facade's DTOs are the wire types:
the write DTO derives `Deserialize` (+ `Serialize`), the response DTO derives
`Serialize`, and both derive [`WireSchema`](../reference/wire-schema.md) so the
OpenAPI document matches what is (de)serialised.

## The rule: DTOs, never domain types

Only DTOs cross the boundary — never a domain entity. The application layer maps
the domain to the DTO at the edge, so internal structure (and anything sensitive)
never leaks onto the wire by accident. The `accounts` example makes this concrete:
its `UserDto` has only `id` and `username`, so a password hash *cannot* be
serialised out — there is nowhere for it to sit.

## Versioning

The contract is versioned at the facade (`v1`, `v2` side by side), so a wire
change is a new version, not a silent break. Within a version, additive DTO
changes (new optional fields) stay compatible; the `WireSchema`-generated document
shows consumers exactly what to expect.

## JSON today

HTTP bodies are JSON. The proto3 document (`seiva proto`) is generated from the
same descriptors for a future gRPC transport; events use the CloudEvents JSON
envelope (see [messaging](messaging-and-events.md)).

## Related

- [Validation](validation.md) · [API documents](api-documents.md) · [Wire schema](../reference/wire-schema.md)

---

*Part of the [Seiva documentation](../index.md).*
