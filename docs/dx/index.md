# dx — the Delonix package manager

`dx` is Delonix's package manager and build tool for Rust. It is
**Cargo-compatible by design**: Cargo and `rustc` still resolve and compile your
code, so the manifests `dx` writes are plain Cargo manifests. On top of that,
`dx` answers the question Cargo leaves to you — *"what am I actually pulling in,
and is it safe?"* — by making every dependency change **explainable**,
**security-checked before it lands**, and **recoverable**.

The binary is `dx`. It is inspired by the clarity of
[uv](https://docs.astral.sh/uv/) for Python, while deliberately respecting
Rust's native mechanisms: `dx` never replaces the resolver or the compiler, it
drives them.

!!! info "Optional, like the rest of Delonix"
    `dx` does not require Seiva, and Seiva does not require `dx`. A Seiva
    project is an ordinary Cargo workspace — you can build and run it with
    **either `dx` or `cargo`**. The tutorials show both in tabs; pick one. `dx`
    adds value when you want explainable, auditable dependency changes, but it
    is never mandatory.

## Why it exists

Cargo is excellent at resolving and building. What it does not do is tell you,
*before* a dependency lands, what that dependency drags in, whether it carries a
known advisory, or whether it runs native build code. `dx` fills exactly that
gap:

- **Explainable changes.** `dx add --plan` shows the impact of an addition —
  the graph it introduces, whether it brings `build.rs`/proc-macros/native
  code, its effect on your MSRV, and the security verdict — *before* writing a
  single line to `Cargo.toml`.
- **Security before execution.** `dx add` resolves the candidate graph in an
  isolated staging area (**without** running any `build.rs`) and runs a security
  gate. The dependency only lands if the verdict is **Approved**.
- **Predictable recovery.** Every mutation is transactional (journal plus
  `tmp + fsync + rename`), so an interrupted operation recovers cleanly on the
  next run; repeating a command is a no-op.

The design rationale lives in the `dx` repository's ADRs — see
[ADR-0015 (positioning and impact plan)](https://github.com/ngolacloud/dx/blob/main/docs/adr/0015-posicionamento-e-plano-de-impacto.md).

## Install

`dx` installs with Cargo, like any other Rust tool. The published crate is
`ngoladx-cli` (the short names were already taken on crates.io), and it provides
the `dx` binary:

```console
$ cargo install ngoladx-cli      # installs the `dx` binary
$ dx --help
```

There is also a one-line installer that uses Cargo underneath:

```console
$ curl -fsSL https://ngolacloud.github.io/dx/install.sh | bash
```

!!! note "Rust import names are unchanged"
    Only the *published crate names* carry the `ngoladx-` prefix. The `dx`
    binary, your `Cargo.toml`, and the Rust import paths are all unaffected —
    `dx` writes plain Cargo manifests.

## A first taste

```console
$ dx new my-lib --lib
$ cd my-lib
$ dx add serde@1 --features derive          # idempotent — repeating is a no-op
$ dx add serde@1 --features derive --dry-run # show the plan, write nothing
$ dx build
```

The generated `Cargo.toml` is pure Cargo: it compiles and tests **without** `dx`
installed. That is a deliberate acceptance criterion — `dx` is a layer over
Cargo, not a fork of it.

!!! warning "Calling `cargo` directly bypasses the gate"
    Because the manifest stays plain Cargo, invoking `cargo` directly skips
    `dx`'s security gate — by design. If your organization relies on the gate,
    enforce `dx` (or the gate) in CI and in a controlled build environment. See
    the `dx` project's
    [security policy](https://github.com/ngolacloud/dx/blob/main/docs/security.md).

## Where to go next

- [Package management](package-management.md) — the real `dx` subcommands for
  adding, removing, building, running, testing, locking and auditing.
- [dx vs Cargo](dx-vs-cargo.md) — a precise command-by-command comparison, what
  `dx` adds, and when to reach for which.
- [Using dx with Seiva](with-seiva.md) — driving a Seiva project end to end with
  `dx`, and how it sits beneath the `seiva` CLI.

---

*Part of the [Seiva documentation](../index.md).*
