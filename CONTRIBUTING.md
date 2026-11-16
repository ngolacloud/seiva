# Contributing to Delonix Seiva

Thank you for considering a contribution. Seiva is a Rust backend framework for
robust, high-criticality systems; it is young, and thoughtful contributions —
from a typo fix to a new adapter — are genuinely welcome.

This guide is short on purpose. The CI gate is the source of truth for whether a
change is ready; everything below just helps you get there.

## Ground rules

- Be respectful. This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- Open an issue before a large change. For a new architectural pattern, boundary
  or public contract, we write an [ADR](docs/adr/) first (see the
  `seiva-community-adr` discipline) — propose it in the issue.
- Keep the DORM boundary intact: the framework core never depends on DORM. DORM
  enters only through the `seiva-data-dorm` gateway behind the `dorm` feature.
  See [CLAUDE.md](CLAUDE.md) and the architecture guide for why this matters.

## Getting started

```sh
git clone https://github.com/ngolacloud/seiva
cd seiva
cargo build --workspace            # DORM-free; needs no database or credentials
cargo run -p seiva-cli --bin seiva -- doctor
```

Rust **1.75 or newer** (the declared MSRV). The default build is DORM-free, so
you need no PostgreSQL instance and no private-repo token to work on the core.

## The checks that must pass

Run these before opening a PR — they are exactly what CI (`.github/workflows/ci.yml`)
enforces:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check                   # supply-chain / licence / advisories gate
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items
```

If you change `seiva-http`, `seiva-openapi` or the CLI, also run the
developer-experience parity checklist; if you touch `seiva-http`, `seiva-auth`
or `seiva-webhooks`, run the OWASP security review. Both live as project skills
in `.claude/skills/`.

## Commits and pull requests

- **Conventional Commits** for subjects (`feat:`, `fix:`, `docs:`, `refactor:`,
  `test:`, `chore:`). Portuguese or English subjects are both fine.
- Branch for your work; `main` is the integration branch (trunk-based, merged via
  PR). Keep PRs focused — one concern per PR reviews faster.
- Describe *what* changed and *why*. Link the issue. If behaviour changed, say how
  you verified it (a test, a reproducible run).
- Add or update a `CHANGELOG.md` entry under `## [Unreleased]` for user-visible
  changes.

## Adding a module or example

A module is **five crates, one per layer** (`domain`, `application`, `facade`,
`infrastructure`, `interface`). Copy the shape from `examples/notes` or
`examples/accounts`, add the crates to the root `Cargo.toml` `members`, and run
`seiva lint` (the architecture fitness functions) before pushing.

## Reporting bugs and requesting features

Use the issue templates. A good bug report includes the Seiva version (or commit),
the Rust version (`rustc --version`), what you expected, what happened, and a
minimal reproduction.

## Licence

By contributing, you agree that your contributions are licensed under the same
dual **MIT OR Apache-2.0** terms as the project.
