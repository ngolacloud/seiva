# Seiva internals

How Seiva works under the hood, and how to work on it — the *internals* and
*open-source project* material.

- [Architecture](../architecture.md) — the dependency rule, the contract, how the
  pieces fit
- [The contract and the macros](the-contract-and-macros.md) — how `#[facade]`,
  `#[controller]` and `#[derive(WireSchema)]` expand (no hidden magic)
- [Architecture Decision Records](adrs.md) — the design history, one decision per ADR
- [Contributing](contributing.md) — the dependency rule, fitness functions, the
  anti-pattern catalogue, the ADR-first workflow
- [The release process](release-process.md) — versioning, publishing, release notes

## Design philosophy

1. **The contract is the source of truth.** You declare a facade; the transports,
   the OpenAPI document and the permission checks are derived — never hand-written.
2. **No hidden magic.** Every macro has a documented hand-written equivalent; run
   `seiva expand` to see it.
3. **The compiler enforces the architecture.** The layering is the crate graph; you
   cannot violate it and still build.
4. **Honest by default.** Generated contracts match what the server accepts;
   placeholders are labelled; capabilities are tracked in the log and the ADRs.
5. **The core never depends on the data layer.** DORM enters only through a feature.

---

*Part of the [Seiva documentation](../index.md).*
