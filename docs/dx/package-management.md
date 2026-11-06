# Package management with dx

This page is a reference to the `dx` subcommands you use to manage a project day
to day: scaffolding, adding and removing dependencies, building, running,
testing, and verifying the dependency graph. Every command here is backed by the
real `dx` CLI; where a command delegates to Cargo, that is called out.

!!! note "Exit codes"
    `dx` uses meaningful exit codes: `0` success · `1` error · `3` a command
    that is planned but not yet implemented · `4` the security gate **blocked**
    a change · `5` the change is **pending** (for example, the advisory database
    is stale) · `6` the gate was **not applicable** and the change landed
    *unverified* · otherwise the exit code of the underlying Cargo invocation.
    Structured output goes to stdout; the plan and progress go to stderr.

## Scaffold a project

`dx new` creates a new project; `dx init` does the same in the current
directory. Both refuse to clobber — `new` rejects a non-empty directory and
`init` refuses if a `Cargo.toml` already exists. Both print the plan first, and
`--dry-run` writes nothing.

```console
$ dx new my-service --bin        # or --lib
$ cd my-service
$ dx init --lib                  # same, but in the current directory
```

## Add a dependency

`dx add` adds or updates a dependency **idempotently**: features are normalized
(order and duplicates do not matter), so repeating the same `add` is a byte-for-byte
no-op.

```console
$ dx add serde@1 --features derive
$ dx add serde@1 --features derive          # no-op: nothing changes
$ dx add serde@1 --features derive --dry-run # print the plan, write nothing
```

The spec is `name` or `name@<req>`. Common options:

| Option | Meaning |
|--------|---------|
| `--features a,b` | enable features (normalized) |
| `--no-default-features` | drop default features |
| `--optional` | optional dependency |
| `--dev` / `--build` | target the `dev-` or `build-dependencies` section |
| `--git <url>` with `--rev` / `--tag` / `--branch` | a git source |
| `--path <dir>` | a local path source |
| `--registry <name>` | an alternative registry (version required) |
| `--manifest-path <Cargo.toml>` | operate on a specific manifest |

`dx add` enforces a few honest rules:

- Changing the **source** of an existing dependency is an error — remove it
  first, then add it again. `dx` never swaps a source silently.
- A `--registry` dependency **without** a version is rejected; use `name@<req>`.
- A name that looks like a typosquat of a popular crate is refused, with a
  suggestion.

### The security gate on `add`

Before the dependency lands, `dx` resolves the candidate graph in an isolated
staging area (`cargo generate-lockfile`, **without** running any `build.rs`) and
runs its security gate:

- **Approved** → the dependency lands.
- **Blocked** (exit `4`) → it does **not** land; the manifest is untouched.
- **Pending** / stale advisory database (exit `5`) → it does **not** land.
- **Not applicable** — for workspace- or path-style graphs — the dependency
  lands but is marked *unverified* (exit `6`); run [`dx audit`](#audit-the-graph)
  afterwards.

Use `--offline` to resolve only from the local cache. The maximum tolerated age
of the advisory database is configurable via `DX_MAX_DB_AGE_SECS`.

!!! info "Why `--plan` is worth it"
    `dx add --plan` shows the full impact *before* anything lands: the graph the
    addition introduces, whether it pulls in `build.rs` / proc-macros / native
    code, its effect on your MSRV, the security verdict, and whether it is served
    from cache or newly fetched. It is the explainable counterpart to a blind
    `cargo add`.

## Remove a dependency

`dx remove` removes a dependency and is idempotent — removing something that is
not present is a no-op.

```console
$ dx remove serde
$ dx remove some-dev-tool --dev
```

Accepts `--dev` / `--build`, `--manifest-path`, and `--dry-run`.

## Build, check, run, format, lint

These delegate to Cargo with a structured argument vector (no shell), inheriting
its stdio and exit code:

```console
$ dx check
$ dx build [--release]
$ dx run [-- <args>]
$ dx fmt [--check]
$ dx lint
```

!!! warning "Passthrough commands do not run the gate"
    Only `dx add` applies the security gate. `dx build`, `check`, `run`, `fmt`
    and `lint` are passthroughs to Cargo and do **not** re-run the gate. Run
    [`dx audit`](#audit-the-graph) (or wire it into CI) to check an existing
    `Cargo.lock`.

### Build profiles and measurement

`dx build --profile` applies LLVM profiles through `cargo --config` — without
touching your `Cargo.toml`:

```console
$ dx build --profile release       # ThinLTO + 16 codegen units (balanced default)
$ dx build --profile throughput    # fat-LTO + 1 codegen unit (opt-in, slow)
$ dx build --profile size          # strips the binary
$ dx build --profile release --native --measure
```

`dev`, `release`, `throughput` and `size` are the available profiles. No profile
ever alters panic, overflow or safety settings. `--native` adds
`target-cpu=native` (with a warning — not for distributed releases). `--measure`
reports build time, peak memory and binary size.

`dx bench-build [--profiles a,b,c]` does a **clean** build per profile and
tabulates wall-clock time, peak memory and binary size so you can compare
profiles honestly.

### Profile-guided optimization

`dx pgo` runs real PGO: it instruments the build, runs a workload (the binary
itself or a command you give), merges with `llvm-profdata`, recompiles with the
profile, and compares.

```console
$ dx pgo --bin my-service -- --some-workload-args
```

!!! note
    `dx pgo` prefers the `llvm-profdata` from the Rust sysroot and reports when a
    version is incompatible. A real performance gain still requires measuring the
    service under load.

## Test with suites and reports

`dx test --suite` runs a named suite and emits a report. The built-in suites are
`unit`, `doc` and `integration` (Cargo's built-ins); you can define more in
`dx.toml` under `[test.suites.<name>]` with a `runner` plus a `kind` of `rust`,
`command` or `load`. A suite name with no configuration is rejected.

```console
$ dx test --suite unit
$ dx test --suite integration --format junit --out target/report.xml
$ dx test --suite smoke --format html --out target/report.html
```

`--format` accepts `terminal`, `json`, `junit` or `html`; add `--no-ansi` for
plain terminal output. JSON output is versioned, HTML is self-contained, and in
every format secrets are scrubbed and HTML/XML are escaped. A runner error never
reports green. `load` suites fail on p95/p99/error-rate/rps thresholds, not just
on the exit code (metrics are read from `DX_LOAD_METRICS` or
`target/dx-load-metrics.json`).

## Verify the dependency graph

### Audit the graph

`dx audit` runs the security gate over the project's `Cargo.lock` and reports
findings by category, the state of the advisory database, and a decision (exit
`0`/`4`/`5`). `dx doctor` diagnoses your toolchain, `cargo-audit`, the freshness
of the advisory database, and policy.

```console
$ dx audit
$ dx doctor
```

### Check integrity without compiling

`dx fetch` verifies the integrity of the locked graph **without compiling** — it
checksums the cached `.crate` files against `Cargo.lock`:

```console
$ dx fetch --locked --extract-check
```

`--locked` requires a `Cargo.lock`. `--extract-check` unpacks each crate into a
temporary directory with safeguards against path traversal, symlink/hardlink
escape and zip-bombs, then discards it. A checksum mismatch or an unsafe
extraction is **blocked** (exit `4`). Packages that were never downloaded are
reported as "not covered".

### Explain the graph

`dx why` and `dx explain` answer *why* a crate, feature or duplicate is in your
graph (built on `cargo tree`).

```console
$ dx why smallvec
```

## Update dependencies, explainably

`dx update` makes updates explainable and recoverable:

```console
$ dx update --plan                 # show version changes; lockfile untouched
$ dx update --security             # run the gate before/after; report what an
                                   # update fixes and what has no safe fix
$ dx update serde --apply --test   # write the new lockfile transactionally,
                                   # validate with the gate (and tests)
```

`--plan` resolves in staging and leaves the lockfile intact. `--security` runs
the gate before and after and reports which advisories an update fixes — and
which have no compatible fix (it explains the block rather than swapping
libraries behind your back). `--apply` writes the updated lockfile
transactionally, validates it with the gate (and `--test` if given), and
**restores** the previous lockfile if validation fails. Naming a package (for
example `serde`) restricts the change to the minimal update of that one package.

## The lockfile and workspaces

`dx` does not introduce a lockfile of its own — it operates on the standard
`Cargo.lock`. `dx fetch --locked` and `dx audit` read it, and `dx update --apply`
rewrites it transactionally.

For **workspaces** (which is what a Seiva project is), note the gate's behavior:
a workspace- or path-style graph makes the `dx add` gate report **not
applicable** — the change lands but is marked unverified (exit `6`) — so run
`dx audit` over the resolved `Cargo.lock` to get a verdict. `dx` takes an
advisory workspace lock while mutating, so concurrent operations do not corrupt
the manifest.

## Pin project tools and capture the environment

`dx tool` pins developer tools (as opposed to library dependencies) per project:

```console
$ dx tool install cargo-nextest@0.9    # pinned in dx-tools.toml, into .dx/tools
$ dx tool run cargo-nextest -- run     # runs only the pinned tool (sha256-checked)
$ dx tool run cargo-nextest --isolated -- run
$ dx tool list
```

`install` records provenance (source, version, sha256) and installs with
`cargo install --locked` into `.dx/tools`. `run` executes **only** the pinned
tool, verifying its sha256 (anti-tamper); it never falls back to a system
binary, and `--isolated` runs it under the sandbox (network off, environment
stripped of secrets).

!!! note "Verified pre-compiled tools are planned"
    By default `dx tool install` compiles the pinned version. Verified
    pre-compiled binaries (via `cargo-binstall` under policy) are a planned
    future path.

`dx environment capture|verify` records a reproducible-enough build environment
(toolchain, target, auxiliary tools, flags, security evidence and the sha256 of
`Cargo.lock`) into `dx-environment.json` to commit and share, then detects
**drift** against the host:

```console
$ dx environment capture
$ dx environment verify            # critical drift fails with exit 4
```

This is environment recording plus drift detection — not byte-for-byte
reproducibility.

## Build offline and ship on-premises

`dx build --isolated` compiles with **real** isolation: the network is turned
off via a user/network namespace, the environment is cleared down to a minimal
allowlist (no secrets leak in), and resource limits are applied. It uses
`--offline` and disables any network-reaching `rustc` wrapper, and reports
exactly what was enforced.

```console
$ dx build --isolated
```

!!! warning "Filesystem isolation is not enforced yet"
    In this version, `dx build --isolated` does not enforce filesystem
    isolation — it reports that as `false`. Network isolation, the stripped
    environment and the resource limits *are* enforced.

`dx bundle` produces a verifiable offline package for air-gapped or on-premises
builds:

```console
$ dx bundle export --out ./offline
$ dx bundle verify ./offline       # recompute hashes; any mismatch fails (exit 4)
```

`export` writes `Cargo.toml`/`Cargo.lock`, `environment.json`, `sbom.cdx.json`, a
vendored `vendor/` tree plus `cargo-config.toml` (via `cargo vendor`), and a
`bundle.json` with a sha256 per file and an aggregate hash. `verify` recomputes
the hashes and fails on any divergence. A verifiable package — again, not
byte-for-byte reproducibility.

## Prepare a release

`dx release prepare` validates in a clean environment (fmt / `clippy -D` / test /
build plus the security gate; and API/semver via `cargo-semver-checks` when it is
present) and produces review artifacts under `target/dx-release/`: a CycloneDX
SBOM, provenance, the topological publish order, and binaries with checksums.

```console
$ dx release prepare
```

!!! important "Prepare is not publish"
    `dx release prepare` does not create tags, change versions, or publish. It is
    idempotent — it only rewrites `target/dx-release/` for review. A stale
    advisory database makes it report **NOT READY**.

---

*Part of the [Seiva documentation](../index.md).*
