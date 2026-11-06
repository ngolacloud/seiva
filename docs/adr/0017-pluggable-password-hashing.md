# ADR 0017 — Pluggable password hashers + a reference credential flow

- Status: Accepted
- Date: 2026-11-06
- Deciders: NgolaCloud
- Follows: ADR 0006

## Context

`seiva-auth` hashes passwords with Argon2id and verifies them, both as **free
functions** hardcoded to `Argon2::default()` (`hash_password`, `verify_password`).
The output is a self-describing PHC string, but verification always runs Argon2id:
a bcrypt/PBKDF2/scrypt hash cannot be checked, and there is no way to add an
algorithm, change parameters as hardware improves, or migrate a legacy hash —
without editing or forking the crate.

Django's model is the reference: `PASSWORD_HASHERS` is an **ordered list** of
hasher backends. The first is used for new passwords; any of them can *verify* an
existing hash (so you can import users from another system); and when a stored
hash was made by a non-preferred hasher or with weaker parameters, Django
transparently **re-hashes it on the next successful login** (`needs_rehash`). You
extend by adding a hasher to the list — you never rewrite the auth code.

`seiva-auth` also ships a `LoginThrottle` (account lockout, OWASP API2), but it is
a bare primitive wired to **nothing**: no code path composes "check lockout →
verify password → record success/failure", so the lockout is not actually in
service for any login.

## Decision

**Introduce a `PasswordHasher` trait and an ordered `PasswordHashers` registry
(Django's `PASSWORD_HASHERS`), and a reference `authenticate_password` flow that
composes the throttle, the registry and rehash-on-login — all additively, keeping
the existing free functions working.**

- `trait PasswordHasher` (`id`, `hash`, `identifies`, `verify`, `needs_rehash`),
  `Send + Sync`, so a registry can live behind an `Arc`.
- `PasswordHashers`: an ordered list; `hashers[0]` is **preferred** (used for new
  hashes). `hash()` uses the preferred; `verify()` routes to the first hasher that
  `identifies` the PHC string (so legacy/other-algorithm hashes verify);
  `needs_rehash()` is true when the stored hash is not the preferred algorithm, or
  the preferred hasher reports its parameters drifted from the current default.
- Ships `Argon2idHasher` as the default/preferred. `PasswordHashers::default()` is
  Argon2id-only; `hash_password`/`verify_password` **delegate** to it, so every
  existing caller (the accounts example, the CLI) is unchanged.
- `authenticate_password(throttle, hashers, key, password, stored_phc, now)` — the
  reference credential flow: refuse if the key is locked out (→ `LockedOut(retry)`),
  verify through the registry, `record_success` + report `needs_rehash` on a match,
  `record_failure` on a mismatch (→ `BadCredentials`). A malformed stored hash is a
  data error (`Auth`), not a guess, and is not counted against the throttle.
  An application's login endpoint calls this, then re-hashes and persists when
  `needs_rehash` is set, and maps `LockedOut` to `429 Retry-After`.

Transport and the user store stay out of scope (ADR 0006, ORM-agnostic): this is
the composable primitive a login endpoint is built from, not an HTTP login view.

## Alternatives considered

1. **Keep the single hardcoded Argon2id.** Simplest, and Argon2id is a good
   default. Rejected: it fails the explicit goal — you cannot add an algorithm,
   tune parameters, or verify an imported/legacy hash without forking the crate,
   and there is no upgrade-on-login path.
2. **An `enum Algorithm { Argon2id, Bcrypt, … }` instead of a trait.** Simpler
   than a trait object. Rejected: a closed enum is not extensible by downstream
   crates — adding an algorithm still means editing `seiva-auth`, which is exactly
   what we are trying to avoid. A trait keeps the extension point open.
3. **Leave the `LoginThrottle` unwired and document how to use it.** Rejected: the
   review found the lockout is not in service precisely because nothing composes
   it with verification; a reference flow closes OWASP API2 for real and gives
   apps a correct default instead of an easy-to-get-wrong recipe.

## Consequences

Easier:

- Add a hasher (bcrypt, scrypt, a future algorithm) by implementing one trait and
  placing it in the list — no edits to the auth code. Import users from another
  system (their hashes verify) and transparently upgrade them on login.
- The `LoginThrottle` is actually in service through `authenticate_password`
  (OWASP API2), with lockout, success-clear and the rehash signal in one place.

Harder / debt accepted (2026-11-06):

- One more public surface to keep stable (the trait + registry + flow). Mitigated
  by keeping the free functions as the simple path and the registry as the
  extensible one.
- The framework still ships only Argon2id; bcrypt/PBKDF2 hashers (and their
  dependencies) are left to the application or a later, separate addition, so the
  default dependency tree does not grow.

Enforcement:

- `cargo test -p seiva-auth`: the registry verifies the preferred **and** a
  second (test) hasher, `needs_rehash` flags a legacy/weaker hash, and
  `authenticate_password` is covered for lockout, success-clears-throttle and
  rehash-on-login. `seiva-security-review` (API2/API3) runs on the change.

## Legibility verdict

Readable without the codebase: an outside Rust developer learns that Seiva adopts
Django's ordered-hasher model (preferred-for-new, verify-any, rehash-on-login) via
a trait + registry, that the existing one-liners keep working, and that a
reference `authenticate_password` finally puts the account-lockout throttle into
service — with the deliberate boundary that the user store and HTTP login view
remain the application's.
