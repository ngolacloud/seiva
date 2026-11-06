# dx vs Cargo

`dx` is **not** a replacement for Cargo — it is a layer over it. Cargo and
`rustc` still resolve and compile; `dx` drives them and adds explainability,
security and recoverability around the dependency lifecycle. This page compares
the two precisely so you can decide which to reach for.

## The one-sentence version

Use `cargo` and you get Rust's resolver and compiler. Use `dx` and you get the
same resolver and compiler, plus a plan before every dependency change, a
security gate that runs before anything is built, and transactional mutations
you can recover from. The `Cargo.toml` is identical either way.

## Command equivalences

| Task | `dx` | `cargo` |
|------|------|---------|
| New project | `dx new <name> [--lib\|--bin]` | `cargo new <name> [--lib\|--bin]` |
| Init in place | `dx init [--lib\|--bin]` | `cargo init [--lib\|--bin]` |
| Add a dependency | `dx add <spec> [--features …]` | `cargo add <spec> [--features …]` |
| Remove a dependency | `dx remove <name>` | `cargo remove <name>` |
| Build | `dx build [--release]` | `cargo build [--release]` |
| Type-check | `dx check` | `cargo check` |
| Run | `dx run [-- <args>]` | `cargo run [-- <args>]` |
| Format | `dx fmt [--check]` | `cargo fmt [--check]` |
| Lint | `dx lint` | `cargo clippy` |
| Test | `dx test --suite <name>` | `cargo test` |
| Update dependencies | `dx update [name] [--plan\|--apply]` | `cargo update [name]` |
| Explain the graph | `dx why <crate>` | `cargo tree -i <crate>` |
| Audit advisories | `dx audit` | `cargo audit` (external) |

`dx build`, `check`, `run`, `fmt`, `lint` and the test runner delegate straight
to Cargo with a structured argument vector, inheriting its stdio and exit code —
so they behave exactly as you expect from Cargo.

## What `dx` adds on top of Cargo

These have **no direct Cargo equivalent** — they are the reason `dx` exists:

- **An impact plan before a change lands** — `dx add --plan` shows the
  introduced graph, whether it brings `build.rs`/proc-macros/native code, MSRV
  effects, the security verdict, and cache-vs-new, *before* writing to
  `Cargo.toml`. `cargo add` just writes.
- **A security gate wired into `add`** — `dx add` resolves the candidate graph
  in isolated staging (**no `build.rs` execution**) and only lands the
  dependency if the verdict is **Approved**; it blocks (exit `4`) or defers
  (exit `5`) otherwise. `cargo add` has no security concept.
- **Idempotent, transactional mutations** — features are normalized, repeating
  an `add`/`remove` is a no-op, and every write is journalled
  (`tmp + fsync + rename`) so an interrupted operation recovers cleanly.
- **Explainable, recoverable updates** — `dx update --security` reports which
  advisories an update fixes and which have no compatible fix; `--apply`
  restores the previous lockfile if validation fails.
- **A built-in gate and integrity tooling** — `dx audit`/`dx doctor` and
  `dx fetch --extract-check` (checksum-vs-lockfile plus safe extraction against
  traversal/symlink/zip-bomb) are part of the tool, not a separate install.
- **Build profiles, measurement and PGO as first-class commands** —
  `dx build --profile …`, `dx bench-build`, `dx pgo`, applied via `cargo
  --config` without editing your `Cargo.toml`.
- **Configurable test suites with reports** — `dx test --suite` with
  `terminal`/`json`/`junit`/`html` output, secret scrubbing, and load-threshold
  evaluation.
- **Project-pinned tools, environment capture, offline bundles, release prep** —
  `dx tool`, `dx environment`, `dx bundle`, `dx release prepare`.

## Where `dx` is Cargo-compatible

- **The manifest is plain Cargo.** `dx` writes ordinary `Cargo.toml` entries.
  A `dx`-scaffolded project compiles and tests with `cargo` and no `dx`
  installed — a deliberate acceptance criterion.
- **The resolver and compiler are Cargo and `rustc`.** `dx` never forks or
  reimplements them.
- **The lockfile is `Cargo.lock`.** `dx` introduces no lockfile of its own.
- **Profiles are applied through `cargo --config`,** not by rewriting your
  manifest.

You can move between the two tools freely on the same project, and mix them —
`dx add` a dependency, then `cargo build`, or vice versa.

## Honest limitations

Be aware of these when choosing `dx`:

- **Calling `cargo` directly bypasses the gate.** Because the manifest stays
  plain Cargo, `cargo add`/`cargo build` skip `dx`'s security gate entirely. If
  the gate matters to you, enforce `dx` (or the gate) in CI and in a controlled
  build environment — `dx` is optional by design.
- **Passthrough commands do not run the gate.** Only `dx add` runs it;
  `dx build`/`run`/`check`/`fmt`/`lint` are pure passthroughs. Use `dx audit`
  (or CI) to check an existing `Cargo.lock`.
- **The gate is "not applicable" on workspaces and path graphs.** In a workspace
  (which a Seiva project is), `dx add` lands the change but marks it *unverified*
  (exit `6`); you get a verdict by running `dx audit` over the resolved
  `Cargo.lock`. See [Using dx with Seiva](with-seiva.md).
- **Filesystem isolation is not enforced** in `dx build --isolated` yet (network
  isolation, a stripped environment and resource limits are).
- **Verified pre-compiled tools are planned.** `dx tool install` compiles the
  pinned version today; `cargo-binstall`-style verified binaries are a future
  path.

## When to use which

| Use `dx` when… | Use `cargo` when… |
|----------------|-------------------|
| You want to see a dependency's impact before it lands (`dx add --plan`). | You already know exactly what you are adding and want the shortest path. |
| You want the security gate to vet additions before they compile. | You are inside a tight inner loop and the gate is enforced elsewhere (CI). |
| You want explainable, recoverable updates (`dx update --security/--apply`). | You just need a quick `cargo update`. |
| You are producing an SBOM, an offline bundle, or preparing a release. | You are relying on another release pipeline. |
| You want pinned, provenance-tracked project tools. | System-installed tools are fine for your setup. |

Because the two share the manifest and the lockfile, this is not an either/or
commitment: adopt `dx` where its guarantees help, and fall back to `cargo` for
anything else.

---

*Part of the [Seiva documentation](../index.md).*
