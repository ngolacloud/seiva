# ADR 0035 — Rotating, revocable refresh tokens with reuse detection

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0019 (fail-closed authorization)

## Context

Seiva verifies tokens, it does not **issue** them. `seiva-identity`'s
`JwtAuthenticator` turns a bearer into a `Principal` only from a cryptographically
verified OIDC/JWT (RS256/ES256, `iss`/`aud`/`exp` checked), and ADR 0006 keeps
token *issuance* "an IdP's job". That is right when an external IdP exists — but a
self-hosted platform like the driver, delonix-meet, **is** its own IdP: it logs a
user in with a password (ADR 0017) and must then hand out a session that survives
longer than a single short-lived access token without re-asking for the password.

A naive answer — one long-lived JWT — is the trap. A stateless JWT cannot be
revoked before it expires, so a long TTL means a stolen token is valid for days
with no server-side off switch, and a short TTL means the user re-authenticates
constantly. The industry answer is a **two-token split**: a short-lived access
token (minutes, stateless, the thing `JwtAuthenticator` already verifies) plus a
long-lived **refresh token** (days) that is revocable server-side and exchanged
for a fresh access token. delonix-meet builds this by hand today; it is 100 %
horizontal and belongs in the framework.

The forces:

- **Revocable by construction.** The value of the refresh token is that logout,
  a password change, or an admin "kill all sessions" actually *ends* access. That
  requires server-side state the access token deliberately does not have.
- **Never trust the token at rest.** The store holds a **hash** of the refresh
  token, never the plaintext — a leaked database must not yield usable sessions,
  exactly as passwords are never stored in the clear (ADR 0017).
- **One-time-use rotation.** Each exchange mints a new refresh token and retires
  the old one. A refresh token is a single-use coupon, not a reusable key, which
  shrinks the window a captured token is good for to one request.
- **Reuse is theft.** Rotation makes stolen tokens *detectable*: if an
  already-rotated token is presented again, either the legitimate client or an
  attacker is replaying a spent token — the whole **family** (session) is
  compromised and must be revoked at once (the OAuth refresh-token-reuse pattern).
- **Pluggable store, fail-closed.** The store is a port, like every other backend
  (ADR 0015/0030); any ambiguous lookup denies (ADR 0019), never grants.

## Decision

Add refresh-token issuance to **`seiva-identity`** (data-layer-free), composing
with the existing `Authenticator`/`JwtAuthenticator` rather than replacing them.

- **`RefreshToken`** — a high-entropy opaque secret (≥ 256 bits from `OsRng`),
  returned to the client **once** at mint time. Its server-side record carries a
  `family_id`, a `token_hash`, the `subject`/`tenant` it binds, `issued_at`,
  `expires_at`, an optional `device`/`session` binding, and a `rotated_at`/used
  marker. The plaintext is never persisted.
- **`RefreshTokenStore`** (port) — `insert`, `find_by_hash(hash) -> Option<Record>`,
  `mark_rotated(id, successor_id)`, `revoke_family(family_id)`. Lookup is **by hash**
  over an indexed column (SHA-256 of the presented secret), so verification is one
  indexed read, not a table scan. `seiva-testing` ships an in-memory double; a
  DORM-backed adapter lives behind the `dorm` feature.
- **`issue(subject, tenant, policy)`** — opens a new `family_id`, mints the first
  refresh token, and pairs it with a short-lived access JWT.
- **`rotate(presented)`** — the one-time-use exchange: hash the presented secret,
  look it up, and
  - **not found / expired** → reject (`401`), no state change;
  - **found but already rotated** → **reuse detected**: `revoke_family` the whole
    session and reject — this is the theft signal;
  - **found and live** → mark it rotated, mint a successor in the *same* family,
    and return a new access + refresh pair.
- **`revoke(family_id)`** / **`revoke_all(subject)`** — logout and "kill my
  sessions"; idempotent (revoking an already-revoked family is a no-op success).
- **`RefreshPolicy`** — per-org knobs: access-token TTL (minutes), refresh TTL
  (days), an absolute session lifetime cap, and whether rotation is mandatory.
  Revocation policy (e.g. revoke-all-on-password-change) is the product's to wire
  via the event bus (ADR 0021).

`tokio`/crypto enter with minimal features; no runtime is linked into the library.
The store adapters are optional features; local-only in-memory is the default.

## Alternatives considered

- **One long-lived JWT, no refresh.** Rejected: a stateless JWT cannot be revoked
  before `exp`, so a stolen token is valid for its whole lifetime with no server
  off switch — the exact failure a platform login must not ship.
- **A server-side session cookie (stateful opaque session) only.** Workable, but
  it discards the stateless, cache-free verification of the access JWT on every
  request. The two-token split keeps hot-path auth stateless (verify the JWT) and
  pays the store cost only at refresh time.
- **Store the refresh token in plaintext (or reversibly encrypted).** Rejected: a
  database leak would hand over live sessions. Hash-at-rest + lookup-by-hash gives
  the same O(1) match without a recoverable secret, mirroring password storage.
- **Rotate but do not detect reuse.** Rejected: rotation alone narrows the window
  but throws away its best signal. Treating a replayed spent token as theft and
  revoking the family is what turns rotation from hygiene into a detector.
- **Issue tokens in a new crate.** Rejected: issuance and verification are two
  halves of one identity story; keeping them in `seiva-identity` lets `issue`
  reuse the JWT signer and the `Principal` model directly.

## Consequences

- **Performance.** Verification stays stateless and cache-free (the access JWT is
  checked exactly as today); the store is touched only at `rotate`/`revoke`, and
  then as a single **hash-indexed** read. The crate links no runtime and is
  zero-cost for a service that uses an external IdP and never issues.
- **Security.** Refresh tokens are **hashed at rest** (a DB leak yields no usable
  session), **one-time-use** (a captured token is good for one request), and
  **reuse-detecting** (a replayed spent token revokes the whole family — the theft
  response). Revocation is server-side and real (logout/password-change/admin
  kill actually end access), and every ambiguous lookup denies — fail-closed,
  consistent with ADR 0019.
- **Resilience.** Revocation is **idempotent** (revoking an already-dead family
  succeeds), so retries and concurrent logout/kill paths converge; the store is a
  **port**, so a single process (in-memory) and a multi-pod deployment (shared
  backend) use the same logic with a different adapter; an absolute session cap
  bounds how long any family can live regardless of refresh traffic.
- delonix-meet deletes its hand-rolled refresh/rotation/reuse plumbing and keeps
  only its policy — which routes mint a session, its org TTLs, and the
  domain events that trigger `revoke_all` — as product configuration.
- Token *verification* against an external IdP (ADR 0006) is unchanged; this adds
  the **issuer** side for platforms that are their own IdP, behind the same
  `Principal`.
