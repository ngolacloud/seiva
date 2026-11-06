# Security

Seiva's posture is **secure by default** — mapped onto the OWASP **API Security
Top 10 (2023)**, the right checklist for an API-first framework. Security is enforced by construction (the type system and the layer
stack), not left to each team.

## Secure by default

Every service built on `router_with_modules` gets, with no configuration:

- **Deny-by-default authentication** ahead of every route (see
  [Authentication](authentication.md)) — the single most important control, not
  hand-rolled.
- **Function-level authorization** — each facade method's `#[requires("…")]` is
  enforced by `permission_guard` from the descriptor (OWASP API5).
- **Object-level authorization** — `Scoped<T>` makes cross-tenant/cross-owner
  access a **compile-time** concern, fail-closed by type (OWASP API1 / BOLA; see
  [Authorization](authorization.md) and [ADR 0005](../adr/0005-object-level-authorization-bola.md)).
- **Secure response headers** on *every* response — including short-circuited ones
  (a `429`, a `401`, a CORS preflight), because the header layer is outermost.
- **A rate limiter** and optional CORS (same-origin unless you add origins).
- **A correlation `request-id`** on every request/response.
- **Validated input** → RFC 9457 `422` (see [Validation](validation.md)); the ORM
  parameterises queries (no SQL injection).
- **Improper-inventory protection (API9)** — `/openapi.json` lists *every* mounted
  facade via link-time discovery, so the documented surface matches the real one.

## The layer order matters

Outermost-first: `request_id` → `security_headers` → `authn` → rate limit → CORS →
body-size limit → route. Headers and request-id sit **outside** authn so a `401`
still carries them; authn sits **outside** the routes so it runs before each
route's guard. And modules are merged **before** layering (`router_with_modules`) —
otherwise module routes would escape all of the above. See
[Middleware and layers](transport/middleware-layers.md).

## OWASP API Security Top 10 (2023) — posture

| | Control in Seiva |
| --- | --- |
| API1 Broken Object Level Auth | `Scoped<T>`, fail-closed by type |
| API2 Broken Authentication | `seiva-identity`, verified JWT/OIDC, deny-by-default |
| API3 Broken Object Property Level Auth | DTOs are explicit wire types; no domain leak |
| API5 Broken Function Level Auth | `#[requires]` + `permission_guard` |
| API7 SSRF | JWKS injected, not fetched from the verifying layer |
| API8 Security Misconfiguration | secure headers always on; AppEnv production guard |
| API9 Improper Inventory | `/openapi.json` from link-time facade discovery |

Run the checklist before any PR touching `seiva-http`, `seiva-auth`,
`seiva-webhooks` or the CI security gate — see the `seiva-security-review` skill.

## Related

- [Authentication](authentication.md) · [Authorization](authorization.md)
- [ADR 0005 — BOLA](../adr/0005-object-level-authorization-bola.md) ·
  [ADR 0006 — Identity](../adr/0006-identity-and-authentication.md)

---

*Part of the [Seiva documentation](../index.md).*
