# The release process

## Versioning

The crates version together on the `0.1` line today. `seiva-ports` — the
trait-only boundary the application layer codes against — is kept SemVer-stable by
policy, so a module's `application` can depend on it without churn; breaking the
ports is a deliberate, documented major change.

## Publishing the crates

A generated project depends on the framework by version (`delonix-seiva = "0.1"`,
…) and compiles standalone via an interim `[patch.crates-io]` until the crates are
on crates.io. Publishing (owner-only, `cargo publish`) in dependency order
(`seiva-ports` → `seiva-macros` → … → `seiva-cli`) is what drops the patch for end
users. Status and the publish-ready set are tracked in
[publishing](../publishing.md).

## Release notes

Changes are recorded as they land (the engineering log is [progress.md](../progress.md));
per-release notes will live under [releases](../releases/index.md) once the `0.1`
line is published. An architectural change is also captured in an [ADR](adrs.md).

## Related

- [Publishing status](../publishing.md) · [Release notes](../releases/index.md) · [ADRs](adrs.md)

---

*Part of the [Seiva documentation](../index.md).*
