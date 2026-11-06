# Testing

Seiva is built so the **important things are testable without a server or a
database**, and so the **architecture itself is tested** — not just the behaviour.

## Unit-testing a use case (no server, no DB)

The application layer codes against trait-only ports, so you test a use case with
in-memory fakes from `seiva-testing`:

```rust
#[tokio::test]
async fn create_persists_and_returns_the_dto() {
    let service = RecordService { repository: FakeRepo::default(), ids: FixedIds };
    let dto = service.create(CreateRecord { title: "hello".into() }).await.unwrap();
    assert_eq!(dto.title, "hello");
}
```

No HTTP, no axum, no framework runtime leaks into the application layer — that is
the whole point of the [layering](../architecture.md).

## Controller / HTTP tests

`seiva-controller-test` proves a `#[seiva::controller]` generates a working,
facade-driven router end to end: guarded routes, versioned paths, RFC 9457 errors,
and that authentication actually applies to merged module routes.

## Architecture fitness functions

The architecture is guarded by tests that fail the build if an invariant is
broken — run them with `seiva lint`, and see the `seiva-pattern-fitness` skill for
the anti-pattern catalogue. Examples in the tree:

- the generated `interface` crate must **not** depend on `infrastructure`
  (the layering rule);
- a generated project must **compile standalone** (`cargo build` → 0) — a fitness
  test actually compiles a scaffolded project + module;
- the generated OpenAPI body must be the DTO directly, not a `cmd` envelope.

A fitness function that *compiles or runs* the thing beats one that only asserts on
strings — it catches drift a text check cannot.

## Contract fidelity tests

Because the OpenAPI/proto documents are generated, they are tested against the real
facade descriptors and the real `WireSchema` DTOs — so the published contract
cannot silently diverge from what the server accepts (see
[API documents](api-documents.md) and [ADR 0007](../adr/0007-wire-schema-contract-fidelity.md)).

## Related

- [Architecture](../architecture.md) · [Tutorial part 5](../getting-started/shop/part-8-testing.md)
- [Contributing](../internals/contributing.md)

---

*Part of the [Seiva documentation](../index.md).*
