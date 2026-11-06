# Using dx with Seiva

A Seiva project is an ordinary Cargo workspace, so you can drive it with either
`dx` or `cargo`. This guide shows the end-to-end loop with `dx`, explains how
`dx` relates to the `seiva` CLI, and documents the `dx`/`cargo` tab convention
you see throughout the tutorials.

!!! info "Two tools, two jobs"
    - **`seiva`** is the framework's own CLI — it scaffolds projects and
      modules, generates the OpenAPI/gRPC contracts, runs the fitness checks,
      and drives the **dev loop** (`seiva start`).
    - **`dx`** is the underlying **package manager and runner** — it adds
      dependencies (with a plan and a gate), builds, runs, tests, audits, and
      packages.

    They are complementary: `seiva` owns the development workflow; `dx` (or
    `cargo`) owns the build and dependency lifecycle underneath it. Neither
    requires the other.

## The tab convention

Every build/run command in the tutorials appears in two tabs — pick the tool you
use. The commands are equivalent; the manifest they operate on is the same plain
Cargo workspace.

=== "dx"

    ```console
    $ dx build
    ```

=== "cargo"

    ```console
    $ cargo build
    ```

## Driving a Seiva project end to end

Start from a scaffolded project (see the [Quickstart](../quickstart.md) for
`seiva project new`). From the project root:

### Build the workspace

=== "dx"

    ```console
    $ dx build            # compile the whole workspace
    $ dx build --release  # optimized build
    ```

=== "cargo"

    ```console
    $ cargo build
    $ cargo build --release
    ```

### Run the API binary

A Seiva project exposes its HTTP service as the `<project>-api` crate. For a
project named `polls-site`, that crate is `polls-site-api`:

=== "dx"

    ```console
    $ seiva start                      # the dev loop (add --watch for hot-reload)
    # or run the API crate directly:
    $ dx run -p polls-site-api
    ```

=== "cargo"

    ```console
    $ seiva start                      # the dev loop (add --watch for hot-reload)
    # or run the API crate directly:
    $ cargo run -p polls-site-api
    ```

`seiva start` is the recommended dev loop — it is the `seiva` CLI's job. When you
want to run exactly one crate, or to be explicit in CI, `dx run -p <project>-api`
(the Cargo `-p` package selector passes straight through) does it. The service
serves `/health`, `/openapi.json` and the interactive docs, exactly as described
in the tutorial.

### Test

=== "dx"

    ```console
    $ dx test --suite unit
    $ dx test --suite integration --format junit --out target/report.xml
    ```

=== "cargo"

    ```console
    $ cargo test
    ```

`dx test --suite` adds named suites and report formats (JUnit/JSON/HTML) on top
of Cargo's test runner — convenient for wiring Seiva's test output into CI.

### Keep the architecture honest

The fitness functions remain a `seiva` responsibility — they are Seiva's layer
and zero-trust checks, not a Cargo concept:

```console
$ seiva lint
```

Run `dx audit` alongside it to get a security verdict over the resolved
`Cargo.lock` (see the gate note below).

## Adding dependencies in a Seiva workspace

Use `dx add` from the project root as usual. Preview first, then let it land:

```console
$ dx add time@0.3 --plan     # show the impact before anything is written
$ dx add time@0.3            # land it (idempotent)
```

!!! warning "The gate is 'not applicable' in a workspace"
    A Seiva project is a Cargo **workspace**, and the `dx add` security gate
    reports **not applicable** for workspace- and path-style graphs: the
    dependency *lands*, but it is marked **unverified** (exit `6`). To get an
    actual security verdict, run `dx audit` over the resolved lockfile after the
    build resolves it:

    ```console
    $ dx build       # resolves Cargo.lock
    $ dx audit       # verdict over the project's Cargo.lock
    ```

    Wiring `dx audit` into CI is the reliable way to enforce the gate for a Seiva
    workspace.

## The Seiva template adapter

`dx` ships a Seiva adapter so it can scaffold and extend a Seiva platform with
**dx-managed composition** — a declarative, anchor-based integration that never
runs a crate's build hooks.

### Scaffold a dx-managed Seiva platform

```console
$ dx new my-platform --template seiva
```

This scaffolds a Seiva platform whose composition is managed by `dx`: a
`src/composition.rs` carrying `// dx:seiva:*` anchors, plus a
`[package.metadata.dx.seiva].composition` marker.

### Add a Seiva extension

When the composition is dx-managed, `dx add` can wire an extension in by anchors
— idempotently and reversibly:

```console
$ dx add seiva-console-admin
```

`dx` adds the dependency through the gate and, if the composition is managed,
integrates the extension by anchors (idempotent, reversible) — **never** running
the crate's hooks, requiring no admin permissions, and setting no passwords. You
then recompile with `dx build` to activate it.

!!! note "Unmanaged composition yields a partial state — honestly"
    If the composition is **not** dx-managed, `dx add seiva-console-admin` still
    adds the dependency but leaves a **pending patch** and reports a partial
    state (exit `6`) rather than claiming an integration it did not perform. You
    then wire the extension in yourself.

## Which tool, when, on a Seiva project

- Reach for **`seiva`** for anything workflow-shaped: `seiva project new`,
  `seiva module new`, `seiva start`, `seiva openapi`/`seiva proto`, `seiva lint`.
- Reach for **`dx`** (or `cargo`) for the build/dependency lifecycle: building,
  running a specific crate, adding/removing dependencies with a plan, auditing,
  testing with reports, and packaging.
- Prefer **`dx add --plan`** over a blind add when bringing in a new crate, and
  run **`dx audit`** (in CI especially) because the gate is only advisory on a
  workspace at `add` time.

For the full `dx` command surface, see [Package management](package-management.md);
for the precise differences from Cargo, see [dx vs Cargo](dx-vs-cargo.md).

---

*Part of the [Seiva documentation](../index.md).*
