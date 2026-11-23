# ADR 0040 — The full OIDC login flow with automatic JWKS rotation (extends `seiva-identity`)

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0019 (fail-closed authorization), ADR 0038 (OAuth 2.1 authorization server)

## Context

`seiva-identity` verifies a bearer, but it does not **log a user in**. ADR 0006
deliberately drew the line: the `JwtAuthenticator` holds a `JwkSet` **by value**,
and "fetching it from the issuer's `jwks_uri` and rotating it is a separate
concern (a future refresh port), kept out so the crate stays network-free and
deterministically testable". That deferral is now the open gap. A static `JwkSet`
means that when an external IdP rotates its signing keys — which every IdP does,
on a schedule — every token signed by the new key is rejected until a human
re-injects the key set. In production that is an outage. The composition root was
left to solve it, and across products it is solved differently or not at all.

The driver needs more than verification: delonix-meet logs users in **through an
external IdP** (its Odoo SSO is the reference — a user's org and directory are
born from the first login). That is the full **OpenID Connect** relying-party
flow: discover the IdP, redirect to it, receive and validate an `id_token`, map
its claims to a `Principal`, and **just-in-time provision** the org/user on first
sight. The authorization-server *half* is ADR 0038; this ADR is the *relying
party* half and, critically, **closes the JWKS-rotation gap ADR 0006 left open**.

The forces:

- **JWKS rotation must be automatic and resilient.** The missing piece is a cache
  keyed by `kid` that refreshes on a schedule and on an unknown-`kid` miss — never
  a fetch per request (a latency and a DoS-amplification trap), never a static set
  that goes stale. When a key rolls, verification must keep working without a
  human.
- **Login is a ceremony, verification is a primitive.** The relying-party flow
  (discovery → authorize+PKCE → callback → `id_token` validation → claim mapping)
  is horizontal mechanism; *which* IdPs are trusted, *how* claims map to roles, and
  *what* org a new login joins are the product's policy.
- **JIT provisioning, fail-closed.** On a first valid login, the org and user are
  created from the verified claims (the Odoo-SSO pattern) — but only through a
  product-owned rule, never by trusting an unvalidated claim to grant tenancy.
- **Only asymmetric algorithms, same as ADR 0006.** RS256/ES256 only, never `HS*`
  or `none`; `iss`/`aud`/`exp`/`nbf` validated; `nonce` and `state` bind the
  round-trip; the JWKS is fetched over TLS from an **allowlisted** issuer (the SSRF
  concern ADR 0006 flagged for the wiring layer, now owned here).
- **Fail-closed when the JWKS is unavailable.** If a refresh fails *and* the
  token's `kid` is unknown, verification is **refused** — a stale-but-valid cached
  key may still serve, but an unknown key with a dead JWKS is never waved through.

## Decision

Extend **`seiva-identity`** with an OIDC relying-party flow and an automatic,
resilient JWKS cache — the `JwtAuthenticator` keeps working from a static set, and
a new layer adds fetch+rotation and the login ceremony on top.

- **Discovery** — a `DiscoveryClient` reads `/.well-known/openid-configuration`
  from an **allowlisted** issuer (TLS only), caching the `jwks_uri`, `authorization`
  and `token` endpoints; the document is cached, not re-fetched per login.
- **Automatic JWKS rotation (the gap closed)** — a `JwksCache` keyed by `kid` that:
  (a) refreshes on a TTL; (b) on a verification miss for an **unknown `kid`**,
  triggers a single bounded refresh (coalesced, so a burst is one fetch);
  (c) serves **stale-while-revalidate** when the issuer is briefly unreachable;
  (d) **fails closed** when the `kid` is unknown *and* the refresh fails. This
  replaces the by-value `JwkSet` of ADR 0006 with a live, self-healing key source —
  with a `JwksSource` port so tests inject a deterministic set and no network.
- **Authorization-code + PKCE login** — a `begin_login` that builds the authorize
  URL (with `state`, `nonce`, PKCE `code_challenge`) and a `complete_login` that
  handles the callback: verifies `state`, exchanges the code at the token endpoint,
  and validates the returned **`id_token`** (signature via the `JwksCache`,
  `iss`/`aud`/`exp`/`nbf`, and the `nonce` against the one minted at `begin_login`).
- **Claim → `Principal` mapping** — the validated `id_token`/`userinfo` claims map
  to the single `Principal { subject, tenant, permissions }` (ADR 0006) through a
  product-configured mapping (which claim is the subject, the tenant, the roles).
- **JIT provisioning** — a `ProvisioningHook` port the product implements: on a
  first-seen subject, it creates the org/user/directory entry (the Odoo-SSO "org
  born from first login" pattern) under the product's **org-entry rules** — the
  framework never grants tenancy from a raw claim.

The network client enters behind a feature; the `JwksSource`/`DiscoveryClient`/
`ProvisioningHook` are ports with in-memory doubles, so the login flow stays
unit-testable without a live IdP. `seiva-oauth` (ADR 0038) can itself be the IdP,
closing the loop for first-party SSO.

## Alternatives considered

- **Keep the static `JwkSet` and re-inject on rotation (status quo, ADR 0006).**
  Rejected: that is a scheduled outage waiting to happen — the IdP rotates, tokens
  fail, a human scrambles. Automatic rotation is table stakes for any real SSO.
- **Fetch the JWKS per request.** Rejected: it adds IdP latency to every call and
  turns the IdP into a single point of failure and a DoS amplifier. A `kid`-keyed
  cache with stale-while-revalidate is both faster and more resilient.
- **Refresh the cache but fail *open* when the JWKS is down.** Rejected outright:
  accepting a token whose key you cannot verify is the cardinal auth sin. Stale
  cached keys may serve; an unknown `kid` with a dead JWKS fails closed (ADR 0019).
- **Build the login flow in each product on top of `seiva-identity`'s verifier.**
  Rejected: that re-derives discovery, PKCE, `nonce`/`state` binding and — most
  dangerously — JWKS rotation per product, each subtly different. The whole point of
  ADR 0006 leaving a "future refresh port" was to land it here, once.

## Consequences

- **Performance.** The JWKS and discovery documents are **cached**, not fetched per
  request (closing ADR 0006's implicit cost); refreshes are coalesced so a key-roll
  burst is a single fetch; the hot verification path stays the in-memory
  `JwtAuthenticator`. Zero-cost for a service that does no SSO.
- **Security.** RS256/ES256 only (never `HS*`/`none`); `iss`/`aud`/`exp`/`nbf`,
  `nonce` and `state` all validated; the JWKS and discovery are fetched over **TLS**
  from an **allowlisted** issuer (the SSRF concern ADR 0006 deferred, now owned);
  JIT provisioning grants tenancy only through the product's org-entry rule, never a
  raw claim. Fail-closed throughout (ADR 0019).
- **Resilience.** JWKS rotation is **resilient**: TTL refresh + unknown-`kid`-miss
  refresh + **stale-while-revalidate** + bounded retry keep verification working
  across an IdP key roll with no human; and when the `kid` is unknown *and* the
  JWKS is unreachable, it **fails closed** rather than trusting an unverifiable
  token — turning ADR 0006's "accepted debt" into a self-healing key source.
- delonix-meet reuses the login flow and mounts its Odoo-SSO org-entry rules as the
  `ProvisioningHook` policy (org and directory born from the first login), deleting
  its hand-rolled discovery/rotation plumbing; every future Seiva platform gets
  resilient external SSO as a configuration, not a build.
- It does **not** turn Seiva into an IdP — that is ADR 0038's authorization server;
  this ADR is the **relying party** that consumes any OIDC IdP (including Seiva's
  own) and finally closes the JWKS-rotation gap ADR 0006 opened.
