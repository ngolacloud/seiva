# The contract and the macros

Seiva's macros follow one rule (ADR 0001): a macro keeps your code as written and
only **adds** generated items, and everything it generates has a documented
hand-written equivalent. Run `seiva expand` to see the output for any input.

## `#[seiva::facade(version = "v1")]`

Applied to a trait. It re-emits the trait unchanged (minus the helper attributes
`#[requires]`/`#[policy]`/`#[cached]`, which it consumes) and adds:

- a zero-sized `<Trait>Facade` type implementing `Facade` with a `const DESCRIPTOR`
  (name, version, methods — each with its `requires` and signature);
- an `inventory::submit!` registering that descriptor for link-time discovery.

The descriptor is the [contract descriptor](../reference/contract-descriptor.md);
nothing is reflective.

## `#[seiva::controller]`

Applied to a facade `impl`. It re-emits the impl unchanged and adds a
`<trait>_router(service) -> axum::Router` builder: one route per method, the
verb/path from the method-name convention, the body read with `Valid<T>`, the
per-route permission read from the descriptor, and the `Ok`/`Err` mapped to JSON /
RFC 9457. See [controllers and routing](../topics/transport/controllers-and-routing.md).

## `#[derive(WireSchema)]`

Applied to a DTO struct. It implements `WireSchema::schema()` (an object schema
composed from the fields) and registers the type by name in the schema inventory,
so the OpenAPI generator resolves its real shape. See
[wire schema](../reference/wire-schema.md).

## Why macros, not reflection

The generated artefacts (OpenAPI, proto, routes, permission checks) are all
derived at compile time from the descriptor and the `WireSchema` impls. There is no
runtime reflection and no central registry to keep in sync — the link-time
inventory discovers every facade and DTO compiled into the binary. That is what
makes the contract the single source of truth.

## Related

- [Contract descriptor](../reference/contract-descriptor.md) · [Wire schema](../reference/wire-schema.md)
- [Facades](../topics/transport/facades.md) · [Architecture](../architecture.md)

---

*Part of the [Seiva documentation](../index.md).*
