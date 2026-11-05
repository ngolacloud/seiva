# The transport layer

> This is the Seiva equivalent of Django's **view layer** — but you never write a
> view or a URLconf. The transport is *generated* from the facade contract.

## The flow

```
facade trait (#[seiva::facade])            ← the single source of truth
      │  generates FacadeDescriptor (name, version, methods, requires)
      ▼
#[seiva::controller] on the impl           → <trait>_router(service): axum::Router
      │
composition root (apps/<name>-api)         → router_with_modules(health, &security, auth, modules)
      ▼
seiva-server serve loop                     → the running service (+ /health, /openapi.json, /docs)
```

## In this section

- [Facades: the contract](facades.md) — declaring the versioned trait; the descriptor
- [Controllers and routing](controllers-and-routing.md) — the `create`/`get`/`list`
  convention, the generated router, per-route permissions
- [Requests and responses](request-and-response.md) — the lifecycle, RFC 9457 `Problem`s, status codes
- [Middleware and the layer stack](middleware-layers.md) — request-id, security
  headers, authn, rate limit — and the order that makes them apply to module routes
- [The application and the composition root](application-and-composition.md) —
  `router_with_modules`, health, the serve loop, graceful shutdown

## The routing convention

The method name chooses the verb and path (the resource is the facade name,
lowercased and pluralised):

| method | HTTP | path | body |
| --- | --- | --- | --- |
| `create` | `POST` | `/{version}/{resource}` | the DTO (201 on success) |
| `get` | `GET` | `/{version}/{resource}/{id}` | — (id in path) |
| `list` | `GET` | `/{version}/{resource}` | — (optional query) |
| *other* | `POST` | `/{version}/{resource}/{name}` | the DTO |

A write body is deserialised **as the parameter DTO directly** (`Valid<T>`), and
the OpenAPI document reflects exactly that — see [API documents](../api-documents.md).

## The one subtlety worth knowing

axum's `.layer` wraps only the routes already present. So merging a module router
*after* applying the framework layers would leave module routes **outside** authn,
the security headers and the rate limit. `router_with_modules` merges the modules
**before** layering — the only order that applies the framework's guarantees to
module routes. See [Middleware and layers](middleware-layers.md).

---

*Part of the [Seiva documentation](../../index.md).*
