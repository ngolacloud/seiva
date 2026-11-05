# The application and the composition root

The **composition root** is `apps/<name>-api` — the service's `main`. It is the
one place that picks concrete adapters and assembles the modules into a running
application. It holds no business logic.

## What `main` does

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    observability::init();                       // structured JSON logs
    let config = ServerConfig::from_process_env()?;   // 12-factor config
    let health = Health::starting();
    let security = HttpSecurity::default();
    let auth = None;                             // or an Authenticator (OIDC/JWT)
    let modules = Router::new();                 // module routers merged here
    // seiva:module-merges (managed by the seiva CLI)
    let app = router_with_modules(health.clone(), &security, auth, modules);
    serve_router(config.http_addr, app, health, shutdown_signal()).await
}
```

The runtime pieces come from `seiva-server`: the serve loop, 12-factor
[configuration](../configuration.md), [observability](../observability.md), and a
graceful drain on `SIGTERM` (health flips `Ready → Draining`, in-flight requests
finish). The framework surface — `/health`, `/openapi.json`, `/docs`, `/admin` —
is assembled by `router_with_modules`.

## Merging modules (auto-wired)

Each module's generated router is merged into `modules` **before** the framework
layers wrap it (the rule in [middleware and layers](middleware-layers.md)). You do
not do this by hand: `seiva module new` inserts the merge (and the module's deps)
at the managed anchors, picking the in-memory adapters:

```rust
let modules = modules.merge(notes_interface::notes_router(
    notes_interface::NotesController::new(
        notes_infrastructure::InMemoryRecordRepository::new(),
        notes_infrastructure::CounterIdGenerator::default(),
    ),
));
```

To go to the DORM data layer, swap the in-memory adapters for DORM-backed ones
behind the `dorm` feature (see [the data layer](../data-layer/index.md)); the
composition root is the only place that names a concrete adapter.

## Emitting contracts

The same binary can print its own contract instead of serving: `SEIVA_EMIT=openapi`
(or `proto`) makes it emit the document from its linked modules and exit — which is
how `seiva openapi`/`seiva proto` document *your* project (see
[API documents](../api-documents.md)).

## Related

- [Middleware and layers](middleware-layers.md) · [Configuration](../configuration.md)
- [Controllers and routing](controllers-and-routing.md) · [Observability](../observability.md)

---

*Part of the [Seiva documentation](../../index.md).*
