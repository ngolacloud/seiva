# ADR 0038 — `seiva-oauth`: an OAuth 2.1 authorization server (Seiva today only verifies JWTs)

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0005 (object-level authorization / BOLA), ADR 0006 (identity and authentication), ADR 0019 (fail-closed authorization), ADR 0035 (rotating refresh tokens)

## Context

Seiva verifies tokens; it does not **mint** them. `seiva-identity` turns a bearer
into a `Principal` only from a cryptographically verified OIDC/JWT (RS256/ES256,
`iss`/`aud`/`exp`/`nbf` checked), and ADR 0006 states token *issuance* "stays out
of Seiva entirely (an IdP's job)". That was the right first cut: a framework
should not hand-roll an authorization server casually. But the driver forces the
question. delonix-meet exposes an **MCP** surface and a first-party web client,
and both need *first-party* tokens issued against the platform's own users,
clients and scopes — there is no external IdP that knows "this meeting-platform
tenant's seats and roles". Today that gap is filled by hand, per product, which
is exactly the shape the upstreaming principle says to lift once, correctly.

An authorization server is horizontal: *any* platform that has its own clients
(an MCP server, a SPA, a mobile app, a service-to-service integration) needs to
issue, scope, introspect and revoke tokens for them. The domain stays in the
product: *which* scopes exist and *what* each guards is policy; the OAuth
**ceremony** is mechanism.

The forces:

- **OAuth 2.1, not 2.0.** The obsolete footguns are removed by construction: no
  implicit flow, no password grant, **PKCE is mandatory** on every
  authorization-code exchange (public *and* confidential clients), exact-string
  redirect-URI matching. Shipping 2.0's optional-PKCE shape would re-import the
  vulnerabilities a reference framework must not.
- **Issuance is now a Seiva concern, verification already is.** The server mints
  its own JWTs (ES256/RS256) and publishes its own JWKS; `seiva-identity`
  verifies them with the code it already has (ADR 0006). One trust root, two
  halves.
- **Deny-by-default, everywhere.** Scopes are an allowlist per client; an
  unrequested or ungranted scope is absent, never implied (ADR 0005/0019).
  Authorization codes are short-lived, single-use, bound to the client and the
  PKCE challenge. A reused or expired code is refused.
- **Refresh is already solved.** ADR 0035 owns rotating, revocable, reuse-detecting
  refresh tokens; the token endpoint issues and rotates them through that crate
  rather than re-deriving the lifecycle.
- **Zero-cost when unused and DORM-free.** A service that issues no tokens links
  no authorization server. The client/consent/grant stores are behind ports, like
  every other Seiva persistence seam.

## Decision

Add a new workspace member **`seiva-oauth`** — a data-layer-free, feature-gated
OAuth **2.1** authorization server, composed at the root.

- **Authorization endpoint** — `GET /oauth/authorize`: validates `client_id`,
  the **exact-match** `redirect_uri`, the requested `scope` subset, `state` and
  (for OIDC, ADR 0040) `nonce`, requires a **PKCE** `code_challenge` +
  `code_challenge_method=S256`, renders a **consent screen**, and on approval
  issues a short-lived single-use **authorization code** bound to the client,
  redirect-URI and PKCE challenge.
- **Token endpoint** — `POST /oauth/token`: the `authorization_code` grant
  (verifying the PKCE `code_verifier`, the code's one-time use and binding) and
  the `refresh_token` grant (delegated to ADR 0035). It also supports the
  `client_credentials` grant for service-to-service callers. It **mints a JWT**
  access token (ES256/RS256) with the granted scopes and the subject/tenant
  claims `seiva-identity` already reads.
- **Client registry** — a `ClientStore` port over **confidential** (with a hashed
  client secret) and **public** (PKCE-only) clients, each carrying its exact
  redirect-URI allowlist and the scopes it may request. Dynamic registration is a
  later increment, not this cut.
- **Scopes as domain policy** — the product declares its scope catalogue; the
  server treats scopes as opaque allowlisted strings, deny-by-default, and stamps
  only the granted subset into the token.
- **Introspection + revocation** — `POST /oauth/introspect` (RFC 7662) answers
  active/scope/subject for opaque or reference tokens; `POST /oauth/revoke`
  (RFC 7009) revokes a refresh or access token, flowing through ADR 0035's store.
- **Own signing keys + JWKS** — a `SigningKeyStore` holds the current and previous
  signing keys; the server publishes `GET /oauth/jwks.json` with a stable `kid`
  per key, so verifiers (including `seiva-identity`, ADR 0040) rotate without
  downtime.

`tokio` enters with `default-features = false`; no runtime is linked into the
library. The stores are ports with in-memory doubles for tests and DORM-backed
adapters behind the `dorm` feature.

## Alternatives considered

- **Keep issuance out of Seiva (status quo, ADR 0006).** Rejected now, not then:
  the driver needs first-party tokens for its MCP and web clients, and "each
  product hand-rolls an authorization server" re-derives the most dangerous code
  in the stack per team. The trait seam ADR 0006 left open is exactly where this
  lands.
- **Implement OAuth 2.0 with optional PKCE.** Rejected: 2.0's implicit and
  password grants and optional PKCE are the removed-for-cause features. A
  reference framework ships 2.1's safe profile or nothing.
- **Opaque tokens + introspection only (no JWT issuance).** Rejected as the
  default: it forces a network round-trip per request and duplicates the verifier
  Seiva already owns. JWTs keep verification stateless (ADR 0006); introspection
  stays available for the opaque/reference case.
- **Fold the authorization server into `seiva-identity`.** Rejected: issuance has
  a client registry, a consent UI, signing-key custody and a revocation store —
  a different resource and failure model than stateless verification. A separate
  crate keeps `seiva-identity` network-free and testable.

## Consequences

- **Performance.** JWT access tokens are verified statelessly by `seiva-identity`
  (no per-request round-trip); the server's own JWKS is cacheable with a stable
  `kid`; introspection is reserved for opaque tokens. The crate links no runtime
  and is zero-cost when a service issues nothing.
- **Security.** PKCE is **mandatory** (S256) on every code exchange; redirect-URIs
  are **exact-string** matched; `state`/`nonce` bind the round-trip; confidential
  clients authenticate with a hashed secret; authorization codes are short-lived,
  single-use and challenge-bound; scopes are **deny-by-default** allowlists
  (ADR 0005/0019); no implicit/password grant exists. Fail-closed throughout.
- **Resilience.** Revocation and introspection give an operator a kill switch;
  signing-key rotation (current+previous `kid`) turns a key roll into a no-downtime
  operation; refresh rotation and reuse-detection are inherited from ADR 0035; a
  compromised client is contained to its own scopes and redirect-URIs.
- delonix-meet stops hand-rolling token issuance for its MCP and web clients and
  mounts its scope catalogue and consent copy as policy on top; the authorization
  server, built once and reviewed once, becomes the trust root that ADR 0040's
  SSO flow and the MCP surface both consume.
- It does **not** replace a full enterprise IdP (no user federation UI, no
  built-in directory): this is the first-party authorization server for a Seiva
  platform's own clients, with OIDC relying-party login (ADR 0040) covering
  external identity.
