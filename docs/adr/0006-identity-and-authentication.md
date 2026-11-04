# ADR 0006 — Identity and authentication: one verified `Principal` for every transport

- Status: Accepted
- Date: 2026-11-04
- Deciders: NgolaCloud
- Follows: ADR 0003, ADR 0005

## Context

Seiva enforces function-level (ADR 0003, `permission_guard`) and object-level
(ADR 0005, `Scoped<T>`) authorization — but both read a `Principal` that
*something upstream* must have produced, and over HTTP **nothing does**.
`permission_guard` falls back to `Principal::anonymous()` when no `Principal` is
in the request extensions, so every guarded route is reachable only by an
authenticated caller the framework never authenticates. The readiness audit
(2026-11-04) made this the top blocker: the single most important control of a
critical platform is left for each team to hand-roll.

Two more forces shape the answer:

- There are already **two** `Principal` types — `seiva-http::guard::Principal`
  (subject, tenant, permissions) and `seiva-mcp::auth::Principal` (subject,
  tenant, can_write) with an `Authenticator` trait that already denies by
  default. They will drift into incompatible auth models unless unified.
- Verifying a JWT is a crypto minefield (`alg: none`, algorithm confusion,
  unvalidated `exp`/`aud`). A reference framework must not hand-roll it, and
  must not reach the network from a layer that has to stay testable.

## Decision

**One transport-free `seiva-identity` crate owns the `Principal` and the
`Authenticator` trait; a `JwtAuthenticator` turns a bearer into a `Principal`
only from a cryptographically verified OIDC/JWT; the HTTP middleware mounts it
ahead of `permission_guard`, denying by default.**

1. **`seiva-identity` (new, transport-free, like `seiva-ports`/`seiva-auth`).**
   It defines the single `Principal { subject, tenant: Option<String>,
   permissions: BTreeSet<String> }` and the `Authenticator` trait
   (`fn authenticate(&self, bearer: &str) -> Result<Principal, AuthError>`,
   **deny by default**). `seiva-http` and `seiva-mcp` depend on it and drop their
   local copies; `seiva-http` re-exports `Principal`/`Scoped` so its surface is
   unchanged. The static `TokenAuthenticator` moves here as the dev/test impl.

2. **`JwtAuthenticator` verifies with the audited `jsonwebtoken` crate (on
   `ring`).** Algorithms are an allowlist of **RS256 and ES256 only** — no HS*
   shared secret, never `alg: none`. It validates `iss`, `aud`, `exp` and `nbf`
   (small leeway) against an injected `jsonwebtoken::jwk::JwkSet`. Claim → Principal
   mapping is configured: `sub` → subject; a configurable tenant claim
   (default `tenant`); permissions from a configurable claim (default
   `permissions` as a string array, with space-delimited `scope` supported).

3. **Key material is injected, not fetched here.** `JwtAuthenticator` holds the
   `JwkSet` by value. Fetching it from the issuer's `jwks_uri` and rotating it is
   a separate concern (a future refresh port), kept out so the crate stays
   network-free and deterministically testable (inject a locally generated key
   set). This also defers the SSRF/issuer-allowlist concern (audit API7) to the
   wiring layer rather than baking a client in.

4. **Deny-by-default, but public routes still serve.** The HTTP middleware,
   mounted ahead of `permission_guard`: a **missing** bearer → `anonymous()`
   (so `/health`, `/docs`, `/openapi.json` stay public and guarded routes 403
   via the existing guard); a **present-but-invalid** bearer → `401` RFC 9457
   `Problem` with `WWW-Authenticate: Bearer`. The guard remains the per-route
   deny-by-default gate.

## Alternatives considered

- **Keep per-transport `Principal`/auth (status quo).** Rejected: the two
  `Principal` types drift, and HTTP has no authenticator at all — the blocker.

- **Symmetric HS256 shared secret.** Rejected: a secret shared across every
  service, no key rotation, no asymmetric trust. A critical multi-service
  platform needs JWKS/asymmetric verification where only the IdP can mint.

- **Hand-roll JWT verification.** Rejected: `alg: none` and algorithm-confusion
  are exactly the footguns a reference framework must not ship; use the audited
  `jsonwebtoken` + `ring`.

- **Opaque tokens + introspection, or session cookies.** Rejected *for the first
  cut*: stateful, needs a store/roundtrip. JWT/JWKS is stateless and fits the
  12-factor runtime. Both remain possible later as another `Authenticator` impl
  behind the same trait — which is the point of the trait.

- **Reject missing tokens with 401 globally in the middleware.** Rejected: it
  kills public routes. Deny-by-default is already enforced per route by
  `permission_guard`; missing → anonymous is the correct split.

## Consequences

- **Easier:** one `Principal` across HTTP, MCP and any future transport (gRPC);
  swapping the IdP is swapping the `Authenticator`; the authz layers (ADR 0003,
  0005) finally have a trustworthy source for subject/tenant/permissions.

- **Accepted debt (2026-11-04):** JWKS fetch + rotation and the issuer allowlist
  (SSRF, audit API7) are deferred — the crate consumes a static `JwkSet` until a
  refresh port lands. Token *issuance* stays out of Seiva entirely (an IdP's
  job). Recorded in `docs/progress.md`.

- **Layering:** `seiva-identity` is transport-free; `seiva-http` and `seiva-mcp`
  depend on it. Its relationship to `seiva-auth` (password hashing, `Permission`)
  is cross-reference, not merge — `seiva-auth` issues/verifies *credentials*,
  `seiva-identity` verifies *tokens* and models the caller.

- **Enforcement:** the algorithm allowlist (RS256/ES256) and mandatory
  `exp`/`aud`/`iss` validation; a test matrix of bad-signature / wrong-`aud` /
  expired / `alg:none` all yielding no `Principal`; the guard stays
  deny-by-default; and a fitness check that no example injects `Principal::new`
  outside `#[cfg(test)]`.
