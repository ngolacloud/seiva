# Release notes

What changed, when. The running engineering log lives in
[progress.md](../progress.md); this page records the per-release summaries.

## 0.1.0 — first public release (2026-11-05)

The first published cut of the `0.1` line. The framework is wired end to end and
green on CI, and the publishable crates are on
[crates.io](https://crates.io/crates/delonix-seiva) at `0.1.0`, so a generated
project compiles standalone against versioned dependencies.

### Highlights

- **One contract per module.** `#[seiva::facade]` captures a module's method
  signatures into a `FacadeDescriptor`; the HTTP controller (`#[seiva::controller]`
  → axum), the OpenAPI 3.1 document and the gRPC `.proto` are all *generated* from
  it — no reflection, everything known at compile time.
- **Clean Architecture enforced by the crate graph.** Five crates per module
  (`domain`, `application`, `facade`, `infrastructure`, `interface`); the
  dependency rule is imposed by the compiler, not by convention. The application
  layer codes only against the SemVer-stable trait boundary `seiva-ports`.
- **A production- and cluster-ready server.** `seiva-server::serve()` composes the
  router, `/openapi.json`, `/health` (`Starting → Ready → Draining`), graceful
  shutdown (ctrl-c + SIGTERM), a worker host and 12-factor config.
- **Batteries.** Argon2id identity (`seiva-auth`), typed 12-factor config
  (`seiva-config`), Standard Webhooks HMAC (`seiva-webhooks`), CQRS dispatch
  (`seiva-cqrs`), CloudEvents + brokers (`seiva-messaging`), durable idempotency,
  distributed rate limiting (Redis), and an admin console served from the module
  contracts.
- **DORM as the default, optional data layer** behind a gateway, kept out of the
  public workspace so the framework crates build with no private credentials.
- **Scaffold.** `seiva project new` / `seiva module new` emit transport-wired
  modules that compile and answer `POST → 201` out of the box.

### Published crates (crates.io, `0.1.0`)

Thirteen framework crates: `delonix-seiva` (the entrypoint), `seiva-ports`,
`seiva-macros`, `seiva-config`, `seiva-auth`, `seiva-webhooks`, `seiva-cqrs`,
`seiva-openapi`, `seiva-http`, `seiva-server`, `seiva-messaging`,
`seiva-identity` and `seiva-testing`. Add the framework to a project with:

```toml
delonix-seiva = "0.1"
```

The DORM-coupled crates (`seiva-data-dorm`, `seiva-outbox-relay`,
`seiva-rag-worker`, `seiva-mcp`) and the demo-coupled `seiva-cli` / `seiva-grpc`
are `publish = false` by intent; see [publishing](../publishing.md) for the
rationale. `seiva-cli` installs with
`cargo install --git https://github.com/ngolacloud/seiva seiva-cli`.

### Performance

On a 20-core i9-13900H over loopback (measured 2026-11-05, caveats in
[benchmarks](../benchmarks.md)), the Rust server served roughly **3.4× the
throughput of FastAPI** and **16× that of Django** on `GET /health`, and **~4.8×
FastAPI / ~15× Django** on a validating JSON `POST`, with a p99 below **0.8 ms**
on both endpoints. Re-measure on your own hardware before quoting a ratio.

### Links

- Crate: <https://crates.io/crates/delonix-seiva>
- API reference (rustdoc): <https://ngolacloud.github.io/seiva/>
- Source: <https://github.com/ngolacloud/seiva>

## Related

- [Engineering log](../progress.md) · [Publishing status](../publishing.md) ·
  [The release process](../internals/release-process.md)

---

*Part of the [Seiva documentation](../index.md), organised like the Django
docs (tutorials · topic guides · reference · how-to).*
