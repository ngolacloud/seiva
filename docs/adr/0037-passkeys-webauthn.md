# ADR 0037 — `seiva-auth-webauthn`: passkeys / FIDO2 via `webauthn-rs`

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0036 (MFA / TOTP)

## Context

TOTP (ADR 0036) raises the bar, but it is still a **shared secret typed by a
human**: a convincing phishing page asks for the six digits and the attacker
replays them in real time. The only factor that closes real-time phishing is
**public-key, origin-bound** authentication — WebAuthn / FIDO2, surfaced to users
as **passkeys**. The private key never leaves the authenticator (platform TPM,
Face/Touch ID, or a security key), the assertion is signed over a server-issued
**challenge**, and the browser binds it to the real site's origin — a phishing
site simply cannot produce a valid assertion. This is where the enterprise identity
story is going, and the driver, delonix-meet, will need it; the ceremony mechanics
are 100 % horizontal.

The forces:

- **Do not hand-roll CBOR/COSE/attestation.** WebAuthn is a dense binary protocol
  (CBOR attestation objects, COSE public keys, CTAP). Parsing it by hand is exactly
  the crypto-minefield ADR 0006 refused for JWTs — we delegate to the audited,
  mature **`webauthn-rs`**.
- **The challenge is one-time and short-lived.** Every ceremony turns on a fresh
  server-minted random challenge; the signed assertion is only valid for *that*
  challenge, once, before a short TTL expires. Replaying a captured assertion must
  fail because the challenge is already spent.
- **Origin / RP-ID binding is the anti-phishing property.** The assertion carries
  the origin the browser actually spoke to; the verifier must pin the expected
  `RP-ID` and origin, so a credential registered for the real site cannot be used
  from a look-alike. This is the property TOTP lacks.
- **A cloned authenticator must be detectable.** Each credential carries a
  monotonically increasing **signature counter**; if an assertion presents a
  counter at or below the stored value, the authenticator may have been cloned and
  the attempt must be refused.
- **It must coexist with the other factors.** A passkey is not always available
  (a borrowed device, a lost key). WebAuthn must compose with TOTP (ADR 0036) as a
  fallback, behind the same `Principal` and step-up seam — never a dead end.

## Decision

Add a new workspace member **`seiva-auth-webauthn`** — data-layer-free,
transport-free, wrapping **`webauthn-rs`** and composing with `seiva-identity`.

- **Registration ceremony** — `begin_registration(subject)` asks `webauthn-rs` for
  a `CreationChallengeResponse`, stores the matching registration state in the
  **challenge store** under a short TTL, and returns the challenge to the browser.
  `finish_registration(subject, credential)` verifies the attestation and persists
  a `StoredCredential { credential_id, public_key, sign_count, aaguid, created_at }`.
- **Authentication ceremony** — `begin_authentication(subject)` mints a
  `RequestChallengeResponse` (again TTL-stored); `finish_authentication(assertion)`
  verifies the signature against the stored public key, checks origin/RP-ID, and
  — critically — verifies the **sign-count has advanced**; a non-increasing counter
  is a possible clone and is **refused**. A success updates the stored counter and
  yields a `Principal` (or a step-up proof, ADR 0036).
- **`ChallengeStore`** (port) — holds in-flight ceremony state keyed by a nonce,
  each with a TTL; a challenge is **consumed once** (removed on finish) so it cannot
  be replayed. `seiva-testing` doubles it; a Redis adapter suits multi-pod (the
  browser may finish on a different pod than it began).
- **`CredentialStore`** (port) — the registered credentials per subject
  (`credential_id`, `public_key`, `sign_count`, `aaguid`). `seiva-testing` doubles
  it; a DORM-backed adapter lives behind the `dorm` feature.
- **`WebAuthnPolicy`** — per-org: which users may/must enrol a passkey, **user
  verification** requirement (presence vs. verified), whether **resident keys**
  (discoverable passkeys, usernameless login) are allowed, and the **attestation**
  conveyance (none by default; direct only where device provenance is required).

Signature verification is cheap asymmetric crypto; no runtime is linked into the
library; the Redis challenge-store adapter is an optional feature.

## Alternatives considered

- **Hand-roll WebAuthn (parse CBOR/COSE directly).** Rejected for the same reason
  ADR 0006 refused hand-rolled JWT verification: the attestation/COSE surface is a
  footgun minefield. `webauthn-rs` is audited and tracks the spec.
- **TOTP only, skip passkeys.** Rejected: TOTP is phishable in real time (the user
  reads the code into a fake page). Only origin-bound public-key auth closes that,
  and it is the direction enterprise identity is moving. We ship **both** and let
  the product pick per policy.
- **Ignore the signature counter.** Rejected: the counter is WebAuthn's built-in
  clone detector; dropping it discards a free integrity signal. A regressing
  counter blocks the attempt.
- **Store the challenge in a signed cookie instead of a server store.** Rejected:
  a server-side store with a TTL gives true one-time consumption and works across
  pods; a cookie is replayable until it expires and complicates multi-pod finish.
- **Fold WebAuthn into `seiva-auth-mfa`.** Rejected: WebAuthn pulls a heavy,
  distinct dependency (`webauthn-rs`) and a different ceremony/credential model. A
  sibling crate keeps each factor's dependencies and surface isolated, both behind
  the one `Principal`.

## Consequences

- **Performance.** Each ceremony is one **asymmetric signature verification** plus
  two small store touches (mint challenge, consume + update counter) — cheap crypto,
  bounded I/O. The crate links no runtime and is zero-cost when a service enrols no
  passkeys.
- **Security.** The challenge is **one-time with a TTL** (a captured assertion is
  spent), the verifier **pins Origin / RP-ID** (phishing-resistant by construction
  — the property TOTP lacks), a **regressing sign-count blocks** the attempt
  (clone detection), attestation is available where device provenance matters, and
  the private key never leaves the authenticator. Fail-closed throughout, consistent
  with ADR 0019.
- **Resilience.** The **challenge store is a port with a TTL**, so an abandoned
  ceremony self-expires and a multi-pod deployment can finish where it did not begin
  (Redis adapter); and WebAuthn **falls back to another factor** — TOTP (ADR 0036)
  — when a passkey is unavailable, so a lost device is never a lock-out. Both stores
  are ports, shared across single-process and multi-pod with different adapters.
- delonix-meet deletes its WebAuthn ceremony plumbing and keeps only its policy —
  which users enrol, its attestation stance, its user-verification requirement — as
  configuration.
- Together, ADRs 0035–0037 make Seiva a full first-party identity provider
  (issue + rotate sessions, TOTP, passkeys) behind the single `Principal` of ADR
  0006, not just a token *verifier*.
