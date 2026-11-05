# Part 5: Testing your module

The generated `notes` module already ships with tests. In this part we see the
three levels Seiva makes easy: use-case unit tests, the architecture fitness
functions, and how to run them.

## Unit-test the use case (no server, no DB)

The application layer codes against ports, so the use case tests with in-memory
fakes from `seiva-testing` — no HTTP, no database:

```rust
#[tokio::test]
async fn create_persists_and_returns_the_dto() {
    let service = RecordService { repository: FakeRepo::default(), ids: FixedIds };
    let dto = service.create(CreateRecord { title: "hello".into() }).await.unwrap();
    assert_eq!(dto.title, "hello");
}
```

Run them:

```console
$ cargo test
```

The generated module includes this test (and a domain-rule test) out of the box.

## Architecture fitness functions

`seiva lint` runs the invariants that keep the architecture honest — for example,
the `interface` layer may not depend on `infrastructure`:

```console
$ seiva lint
```

These fail the build when a layering rule is broken, so the five-layer design
cannot erode (see the `seiva-pattern-fitness` catalogue).

## Controller / HTTP tests

The framework's own `seiva-controller-test` proves a `#[seiva::controller]`
generates a working, guarded, versioned router end to end — the pattern to copy
when you want to test a module's routes with a test client.

## The principle

Put input validation in `Validate`, business rules in the `domain` (unit-tested
with no dependencies), and keep the application layer transport-free so its tests
need nothing but fakes. The expensive things (a database, a server) are for the
few integration tests, not the many unit tests. See [Testing](../../topics/testing.md).

## Next

[Part 6: Packaging and deployment](part-6-packaging-and-deploy.md).

---

*Part of the [Seiva documentation](../../index.md).*
