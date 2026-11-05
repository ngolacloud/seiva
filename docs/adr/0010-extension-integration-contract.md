# ADR 0010 — The extension integration contract (`dx add` / `seiva ext add`)

- Status: Accepted
- Date: 2026-11-05
- Deciders: NgolaCloud
- Follows: ADR 0003, ADR 0008, ADR 0009

## Context

ADR 0008/0009 made the admin console an optional, per-module-registered
extension. What remained was the **install** experience the brief asks for:

```
dx add seiva-console-admin
seiva module new inventory --admin
seiva start
```

A declared dependency in `Cargo.toml` is **not** enough to insert routers: the
composition root must link the crate *and* compose it (mount its router) and
recompile. The brief is explicit about the rules:

- Integration must be **declarative and auditable**: versioned metadata and
  operations (dependency, feature, import/anchor, mount, config). No `build.rs`
  or install script may run to discover how to install an extension.
- It must be **idempotent**: running it twice duplicates nothing.
- For a project the tool does **not** recognise (no managed composition point),
  it must emit the **exact patch** and report integration as *pending*, not
  guess.
- DX stays an independent tool, outside this repo; we define the **contract** it
  (and the Cargo path) consume, without copying DX's dependency manager.

## Decision

**An extension ships a declarative manifest — the integration contract — and the
composition root exposes a managed composition point. A tool reads the manifest
and applies two edits (add dependency, insert mount at the anchor); it executes
no install script. `seiva ext add` applies it for first-party extensions; the
independent DX reads the same manifest shape; the Cargo equivalent is the same
two edits by hand.**

- **Managed composition point.** `seiva project new` now emits two managed
  anchors in the composition root: `# seiva:extension-deps` in the app
  `Cargo.toml` `[dependencies]`, and `// seiva:extensions` in `main.rs`
  immediately after `let app = router_with_modules(…);`. The mount statement
  shadows `app` (`let app = seiva_console_admin::mount(app);`).
- **Manifest shape.** `ExtensionManifest` (in `delonix_seiva::scaffold`):
  `crate_name`, `version_req`, `mount_stmt`, `summary`. The console ships the
  same data as a declarative `seiva-extension.toml` (`[dependency]`, `[compose]`)
  — the artifact DX reads. A test asserts the shipped TOML and the built-in
  constant agree, so the two install paths wire the crate identically.
- **Application is pure and idempotent.** `add_extension_to_app_manifest` and
  `mount_extension_in_main` insert at the anchors and are no-ops when the
  dependency/mount is already present (so re-running changes nothing). A missing
  anchor is a `MissingAnchor` error — the signal that the project is not a
  recognised managed root.
- **`seiva ext add <name>`.** Resolves the first-party manifest, computes **both**
  edits before writing either (so a missing anchor leaves the project untouched),
  writes them, and tells the user to recompile. For an unrecognised project it
  prints the exact dependency line and mount statement to apply by hand and exits
  with a distinct code, reporting integration as pending. It runs no build or
  install script.
- **First-party registry is tiny and static.** `seiva` knows only its own
  extensions (`seiva-console-admin`); it fetches and executes nothing to discover
  how to install. Third-party extensions ship `seiva-extension.toml` for DX.

## Consequences

**Positive.**

- `dx add seiva-console-admin` (DX) and `seiva ext add seiva-console-admin`
  (Cargo-only) share one contract and produce the same wiring; the manual Cargo
  steps are documented and match.
- Installation is explicit, auditable and recompiled — never a hidden hook.
- Re-running is safe; a declared-but-unmounted dependency never silently exposes
  routes.
- An unmanaged project still gets an exact, copy-pasteable patch.

**Negative / cost.**

- The composition root must carry the managed anchors; a hand-written `main.rs`
  that removed them falls back to the manual patch (by design).
- `seiva`'s built-in extension list is first-party only; third-party extensions
  rely on DX reading their `seiva-extension.toml` (or the user applying the patch).

## Alternatives considered

- **`build.rs`/install script that wires itself.** Rejected outright by the
  brief: no script runs to discover or perform installation.
- **Dependency alone mounts routes (via a linker-discovered plugin).** Rejected:
  it makes exposure implicit and un-auditable; ADR 0003's explicit-composition
  rule and ADR 0009's authorization gate both require the mount to be deliberate.
- **A full package-manager in `seiva`.** Rejected: DX already owns dependency
  management (a separate product); `seiva` defines only the integration contract.
