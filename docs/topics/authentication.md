# Authentication

> Seiva's equivalent of Django's **authentication system** — but verification-first
> and transport-agnostic.

Unlike FastAPI (which gives you OAuth2/JWT *primitives* and leaves the rest to
you), Seiva ships a **deny-by-default** identity layer that turns a bearer token
into a cryptographically verified `Principal` before any route's permission check
runs. The design is recorded in [ADR 0006](../adr/0006-identity-and-authentication.md).

## One `Principal`, one `Authenticator`

A single transport-free crate, `seiva-identity`, owns the model so HTTP and MCP
cannot drift into incompatible auth:

```rust
pub struct Principal {
    pub subject: String,
    pub tenant: Option<String>,
    pub permissions: BTreeSet<String>,
}

pub trait Authenticator {
    fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError>; // deny by default
}
```

- **`TokenAuthenticator`** — a static map, for dev and tests.
- **`JwtAuthenticator`** — verifies an OIDC/JWT with the audited `jsonwebtoken`
  crate (on `ring`). Algorithms are an **allowlist of RS256 and ES256 only** — no
  HS\* shared secret, never `alg: none`. It validates `iss`, `aud`, `exp`, `nbf`
  against an injected `JwkSet`, and maps claims → `Principal` (`sub` → subject; a
  configurable tenant claim; permissions from a configurable claim, with
  space-delimited `scope` supported).

Key material is **injected, not fetched** here — the crate stays network-free and
deterministically testable; JWKS fetching/rotation and issuer allowlisting (SSRF,
OWASP API7) belong to the wiring layer.

## How it behaves on the wire

Mounted ahead of the permission guard by `router_with_auth` /
`router_with_modules`:

| Request | Result |
| --- | --- |
| **No** bearer | treated as `anonymous()` — public routes serve, guarded routes `403` |
| **Invalid** bearer | `401` with `WWW-Authenticate` |
| **Valid** bearer | `Principal` from verified claims; the route's `#[requires(..)]` is checked |

## Wiring it in a service

`seiva-server` reads OIDC config from the environment and builds a
`JwtAuthenticator` from the JWKS file, mounting `authn` for you:

```
SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
SEIVA_OIDC_ISSUER=https://id.example.com/
SEIVA_OIDC_AUDIENCE=my-api
# optional: SEIVA_OIDC_TENANT_CLAIM, SEIVA_OIDC_PERMISSIONS_CLAIM
```

A partial trio fails the boot (fail-closed). Proven live: no token → `200`
(anonymous), bad bearer → `401`, a valid RS256 JWT → `200`.

## Related

- [Authorization](authorization.md) — object-level, fail-closed `Scoped<T>` (BOLA)
- [Security](security.md) — the wider secure-by-default posture
- [Configuration reference](../reference/settings.md) — the OIDC variables
- [ADR 0006 — Identity and authentication](../adr/0006-identity-and-authentication.md)

---

*Part of the [Seiva documentation](../index.md).*
