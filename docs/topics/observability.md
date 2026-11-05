# Logging and observability

A Seiva service is observable by default: structured logs, a correlation id on
every request, and tracing spans — set up by the runtime, not by each team.

## Structured logs

The composition root calls `observability::init()` **first**, before anything
else, installing structured JSON logging to stdout:

```rust
observability::init();
tracing::info!(env = %config.env, http = %config.http_addr, "notes-app starting");
```

Logs are JSON on stdout — exactly what a container platform collects. Use the
`tracing` macros (`info!`, `warn!`, `error!`) with structured fields (`%value` for
Display, `?value` for Debug); avoid logging secrets or request bodies.

> Note: the `SEIVA_EMIT` path (`seiva openapi`/`proto`) runs *before*
> `observability::init()` so the emitted document is the only thing on stdout.

## Request correlation

The outermost `request_id` layer assigns every request a correlation id and
returns it in a response header (`REQUEST_ID_HEADER`). Because it is the outermost
layer, **every** response carries it — including short-circuited ones (a `429`, a
`401`, a CORS preflight). Carry that id into your logs to trace a request end to
end. See [middleware and the layer stack](transport/middleware-layers.md).

## Tracing spans

The framework and the worker runtime emit `tracing` spans (near-zero cost when no
subscriber is installed). `observability::init()` installs a subscriber; point it
at your collector in production. Spans give you latency and causality without
hand-rolled timing.

## Health as an operational signal

`/health/live` and `/health/ready` expose the `Starting → Ready → Draining`
lifecycle. On `SIGTERM` the service flips to `Draining` and finishes in-flight
requests (graceful shutdown), so a rollout drains cleanly. See
[the application and composition root](transport/application-and-composition.md).

## Related

- [Middleware and the layer stack](transport/middleware-layers.md) · [Configuration](configuration.md)
- [Performance & cloud-native](performance.md) · [Security](security.md)

---

*Part of the [Seiva documentation](../index.md).*
