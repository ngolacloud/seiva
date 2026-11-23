# ADR 0036 — `seiva-auth-mfa`: TOTP second factor with backup codes and step-up

- Status: Proposed
- Date: 2026-11-23
- Deciders: NgolaCloud
- Follows: ADR 0006 (identity and authentication), ADR 0015 (distributed rate limit), ADR 0017 (pluggable password hashing)

## Context

Seiva authenticates a caller with **one** factor: a verified JWT (ADR 0006) or a
password through the reference `authenticate_password` flow (ADR 0017). A single
factor means a leaked or phished password is a full account takeover — the control
every enterprise buyer, and every compliance regime, asks for first is a **second
factor**. The driver, delonix-meet, hand-builds TOTP enrolment and verification;
it is pure horizontal mechanism (RFC 6238 is the same for any product) and belongs
in the framework.

TOTP (a time-based one-time password, the thing an authenticator app shows) is the
right first second-factor: no SMS cost or SIM-swap exposure, no hardware
dependency, works offline, and interoperates with every authenticator app via a
single `otpauth://` provisioning URI. But it carries sharp edges a framework must
get right once, not leave each team to rediscover.

The forces:

- **The shared secret is a credential.** The TOTP secret is as sensitive as a
  password — anyone holding it can mint valid codes forever. It must be
  **encrypted at rest**, never logged, and never returned after enrolment.
- **Clock skew is real, replay is the threat.** A verifier must accept a small
  window of adjacent time-steps (phones drift), but a code is only one-time: the
  **same** code must not be accepted twice (an attacker shoulder-surfing a code
  has ~30 s otherwise). The last accepted counter must be remembered.
- **Codes are guessable at scale.** Six digits is a one-in-a-million guess; without
  a throttle an attacker simply tries. MFA verification must sit behind the same
  rate limit as login (ADR 0015).
- **Losing the phone must not lock the account.** Enrolment must mint **backup /
  recovery codes** — one-time, hashed at rest like passwords (ADR 0017) — so a lost
  device is a recovery, not a support ticket into account deletion.
- **Some actions deserve a fresh challenge.** Holding a session is not consent to
  delete the org. **Step-up** re-challenges the second factor for a sensitive
  action even inside a live session.

## Decision

Add a new workspace member **`seiva-auth-mfa`** — data-layer-free, transport-free,
composing with `seiva-identity` and `seiva-auth`.

- **Enrolment** — `begin_enrolment(subject)` draws a random TOTP secret (≥ 160
  bits, `OsRng`) and returns the one-time **provisioning URI**
  (`otpauth://totp/{issuer}:{account}?secret=...&issuer=...`) for the app to show
  as a QR. `confirm_enrolment(code)` requires one valid code before the factor is
  armed (proving the secret was stored correctly), then persists the secret
  **encrypted**.
- **Verification** — `verify(subject, code, now)` computes the RFC 6238 HMAC over
  the current time-step and an allowlist of ±N adjacent steps (`skew_window`,
  default ±1), and **rejects any step at or below the last accepted counter**
  (anti-replay). A match advances the stored counter.
- **`MfaSecretStore`** / **`BackupCodeStore`** (ports) — hold the encrypted secret,
  the last-accepted counter, and the hashed backup codes. `seiva-testing` doubles
  them; a DORM-backed adapter lives behind the `dorm` feature.
- **Backup codes** — `issue_backup_codes(n)` returns `n` high-entropy codes **once**
  and stores only their hashes (via `seiva-auth`'s `PasswordHashers`, reusing ADR
  0017). `redeem_backup_code(code)` is one-time: a redeemed code is marked spent.
- **Step-up** — a `StepUpChallenge { subject, action, expires_at }` the product
  mints before a sensitive facade method; `verify_step_up` consumes it with a fresh
  TOTP or backup code, returning a short-lived proof the route checks.
- **`MfaPolicy`** — per-org: which routes/actions require MFA, whether it is
  mandatory org-wide, the skew window, and the step-up TTL. *Which* actions are
  sensitive is the product's policy; the mechanism is the framework's.

Verification is cheap HMAC; the throttle is `seiva-http`'s rate limiter keyed on
subject (ADR 0015). The crate links no runtime and is zero-cost when unused.

## Alternatives considered

- **SMS / email one-time codes.** Rejected as the first factor: SMS carries SIM-swap
  and interception risk and a per-message cost; email is as weak as the email
  account. TOTP is offline, free, and phishing-resistant to bulk replay. (A later
  channel can implement the same port.)
- **No anti-replay, accept any code in the window.** Rejected: a code glimpsed over
  a shoulder stays valid for the whole step. Remembering the last accepted counter
  is what makes a one-time password actually one-time.
- **Store the TOTP secret in plaintext.** Rejected: a database leak would let an
  attacker mint valid codes indefinitely — the secret is a credential and is
  encrypted at rest, like the password hashes beside it.
- **No backup codes (re-enrol on device loss).** Rejected: a lost phone would mean
  an identity-proofing support flow or account loss. Hashed one-time recovery codes
  are the standard, self-service escape hatch.
- **Put TOTP inside `seiva-identity`.** Rejected: MFA is a distinct, optional
  capability with its own stores and dependencies (HMAC, QR). A separate crate
  keeps `seiva-identity` lean and lets MFA evolve independently, mirroring how
  realtime (ADR 0031) split from `seiva-http`.

## Consequences

- **Performance.** Verification is a handful of **HMAC** computations over a tiny
  step window — microseconds, no I/O beyond one store read. The crate links no
  runtime and is zero-cost for a service that does not enrol MFA.
- **Security.** The TOTP secret is **encrypted at rest** and never returned after
  enrolment; **anti-replay** (skew window + last-accepted counter) makes each code
  truly one-time; verification sits behind the **rate limit** (ADR 0015) so codes
  cannot be brute-forced; backup codes are **hashed, one-time** (ADR 0017); and
  **step-up** re-proves the factor for sensitive actions inside a live session —
  defence in depth, fail-closed.
- **Resilience.** **Recovery codes** mean a lost device is self-service, not an
  account-loss incident; the secret/counter/backup stores are **ports**, so a
  single process and a multi-pod deployment share one logic with different adapters;
  a confirm-before-arm enrolment prevents locking a user out with a mis-stored
  secret.
- delonix-meet deletes its hand-rolled TOTP code and keeps only its policy — which
  actions demand MFA, whether its orgs mandate it — as configuration.
- The crate is the first in an identity-factor family: the passkey/WebAuthn factor
  (ADR 0037) composes with it (TOTP is the fallback when a passkey is unavailable),
  behind the same `Principal` and step-up seam.
