# Contributing

How to work on the framework itself.

## The dependency rule is law

A module is five layer crates and the compiler enforces the direction
(`interface → application → domain`; `infrastructure → application`; the facade is
the contract). You cannot violate the layering and still build. Keep it that way:
the application layer names no transport or database item; the interface never
depends on infrastructure.

## Fitness functions guard invariants

Architectural rules are tests, not conventions. Run them locally:

```console
$ seiva lint        # the architecture fitness functions + anti-pattern catalogue
$ cargo test        # unit + integration tests
```

When you add an invariant, add a fitness function for it — one that *exercises*
the thing where possible (compiles/runs it), not just asserts strings. See
[how to write a fitness function](../howto/write-a-fitness-function.md) and the
`seiva-pattern-fitness` catalogue.

## An architectural change lands in an ADR first

A change that introduces or alters a pattern, boundary or public contract is
recorded in an [ADR](adrs.md) (context, decision, alternatives, consequences)
before it is implemented — so the *why* survives. The `seiva-community-adr`
checklist covers this.

## Secure by default, measured

Changes to `seiva-http`, `seiva-auth`, `seiva-webhooks` or the security gate run
through the OWASP API Top 10 checklist (`seiva-security-review`). Prove behaviour
by driving it, not only by unit tests — the live round-trips in this repo (auth,
the DORM data path, the admin) are the model.

## Formatting and style

`cargo fmt` and `cargo clippy` clean before committing. Match the surrounding
code's comment density and idiom; the generated-code templates are hand-formatted
on purpose.

## Related

- [Architecture](../architecture.md) · [ADRs](adrs.md) · [The contract and the macros](the-contract-and-macros.md)

---

*Part of the [Seiva documentation](../index.md).*
