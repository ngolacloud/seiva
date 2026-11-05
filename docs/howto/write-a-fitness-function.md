# How to write a fitness function

A **fitness function** is a test that guards an architectural invariant and fails
the build when it is broken — so a rule holds over time without relying on review.

## When to write one

When you decide a property must *always* hold: a layering rule, a security
posture, a contract shape, a "no stub left behind". If it matters enough to state,
state it as a test.

## The shape

A fitness function asserts on a fact that is cheap to compute and unambiguous. The
strongest ones **exercise** the thing (compile it, run it, drive the flow) rather
than asserting on strings — they catch drift a text check cannot. Examples already
in the tree:

- the generated `interface` crate must **not** depend on `infrastructure` (the
  layering rule) — a string check on the generated manifest;
- a generated project must **compile standalone** — the test scaffolds a project +
  module and runs `cargo build`;
- a write body's OpenAPI must be the DTO directly (no `cmd` envelope) — asserted
  against the real generated document.

## Write it as a normal test

Put it next to the code it guards, as a `#[test]`/`#[tokio::test]`. Make the
failure message explain the invariant, so a future contributor who trips it learns
*why*, not just *that*.

## Run them

```console
$ seiva lint        # the architecture fitness functions
$ cargo test        # the unit + integration tests, including yours
```

See the `seiva-pattern-fitness` anti-pattern catalogue for the rules `seiva lint`
enforces, and [Testing](../topics/testing.md).

## Related

- [Testing](../topics/testing.md) · [Architecture](../architecture.md)

---

*Part of the [Seiva documentation](../index.md).*
