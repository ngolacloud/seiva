# LLVM release profile

Seiva tunes the Cargo **release** profile (§9) to lean on LLVM through
`rustc`/Cargo — no hand-rolled build magic. The knobs, in the workspace
`Cargo.toml`:

```toml
[profile.release]
lto = "thin"        # cross-crate optimisation (ThinLTO: most of the win, far
                    # cheaper than fat LTO)
codegen-units = 1   # one unit per crate so LLVM sees the whole crate at once
strip = true        # drop symbols from the shipped artefact
```

`panic` stays `unwind` (tests and any `catch_unwind` boundary keep working), and
there is **deliberately no `target-cpu=native`**: the default release binary must
stay portable across the hosts a PaaS schedules it on. Tune per-host only when you
control the target:

```console
$ RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Measurement

The `seiva` CLI binary, built from scratch twice on one host (20-core x86-64,
`rustc 1.96.0`), release profile, default `opt-level = 3`:

| Profile | Binary size | Clean build (wall) |
|---|---:|---:|
| Baseline (`lto = false`, `codegen-units = 16`, no strip) | 6334 KiB | 16 s |
| Tuned (`lto = "thin"`, `codegen-units = 1`, `strip`) | **1680 KiB** | 21 s |
| Δ | **−73 %** | **+31 %** |

The size drop is dominated by `strip` (debug/symbol tables), with ThinLTO +
`codegen-units = 1` adding cross-crate dead-code elimination; those two also let
LLVM optimise across crate boundaries for runtime, at the documented ~31 % longer
clean build. These are single-host figures — treat them as indicative, not a
guarantee; re-measure on your target. RSS and throughput of the *running service*
belong to the production load benchmark (PostgreSQL + auth + idempotency +
replicas), not this compile-time table.

## PGO (opt-in)

Profile-Guided Optimisation is **not** baked into the default profile: it needs a
representative workload and a matching toolchain, and a bad profile can regress.
When you have a representative load, the standard two-pass flow applies
(`-Cprofile-generate` → run the workload → `-Cprofile-use`); keep the generated
profile out of the repo and the binary portable unless you also pin the target
CPU per host.

## What this is not

A smaller, cross-crate-optimised binary is a *build* property. It does not, on its
own, prove the secure service is fast under load — auth, idempotency, the database
and observability all cost — so the production benchmark keeps auth/idempotency
**on** and reports p50/p95/p99, throughput, errors, CPU and memory separately.
