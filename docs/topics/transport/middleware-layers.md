# Middleware and the layer stack

Every Seiva service wraps its routes in a fixed stack of transport middleware,
applied by `router_with_modules`. The order is deliberate and secure-by-default.

## The order (outermost first)

```
request_id → security_headers → authn → rate limit → CORS → body-size limit → route
```

- **`request_id`** and **`security_headers`** are the **outermost** layers on
  purpose: every response carries a correlation id and the secure headers (CSP,
  `nosniff`, HSTS, …) **even when an outer layer short-circuits** — a rate-limit
  `429`, an auth `401`, a CORS preflight. (Mounted inside the rate limiter, a
  `429` once went out naked — measured and fixed.)
- **`authn`** sits inside the header layers (so a `401` still carries them) but
  **outside** the routes (so it runs before each route's `permission_guard`). See
  [Authentication](../authentication.md).
- **rate limit**, **CORS** (only when origins are configured), and the **body-size
  limit** (1 MiB default) follow.

## The one rule that matters: merge before layering

axum's `Router::layer` wraps only the routes already present. So merging module
routes *after* applying the layers would leave them **outside** every middleware —
no authn, no security headers, no rate limit. `router_with_modules` merges the
modules **first**, then layers — the only order that applies the framework's
guarantees to module routes:

```rust
let modules = Router::new().merge(notes_interface::notes_router(/* … */));
let app = router_with_modules(health.clone(), &security, auth, modules);
```

`seiva module new` auto-wires each module into this builder, so you get the order
right by default. See [the application and composition root](application-and-composition.md).

## Configuring the posture

`HttpSecurity` controls the transport-level (OWASP API8) defences: the response
headers are always on; CORS is mounted only when you set allowed origins; the rate
limit is set per `HttpSecurity`. The default is secure and same-origin. See
[Security](../security.md).

## Related

- [The application and composition root](application-and-composition.md)
- [Security](../security.md) · [Authentication](../authentication.md)

---

*Part of the [Seiva documentation](../../index.md).*
