# ADR 0049 — `seiva-kms`: envelope encryption with a guardian-held key, and secrets bound to their context

- Status: Accepted
- Date: 2026-11-27
- Deciders: NgolaCloud
- Follows: ADR 0002 (core ports and test doubles), ADR 0012 (cloud-native health probes), ADR 0047 (mutual TLS, the precedent for a shared capability moving into Seiva)

## Context

Seiva services keep secrets of their own: TOTP seeds (ADR 0036), OAuth client
secrets, LDAP bind passwords, SSH and WinRM credentials, the private key of an
internal CA, webhook signing secrets. Without a shared answer each project
encrypts them its own way, usually with one key in an environment variable. That
key cannot rotate without making every stored secret unreadable, and a database
backup is as good as the plaintext to whoever also has the environment.

Two products built on Seiva already met this:

- **dxstorage** solved it (F8 `4e23bc7`, F11 `fb98cf6`, about 2 000 lines with
  tests): a `Wrapper` port over HashiCorp Vault's transit engine and AWS, Google
  Cloud and Azure KMSs; a ring of data keys (the current one seals, earlier ones
  only open), rotation with re-sealing and retirement; secrets sealed as
  `kms1:<key-id>:<base64>` with AES-256-GCM. It is coupled to the application
  (its engine's configuration, `anyhow`, its cluster store), so nobody else can
  depend on it, and it binds no associated data: a sealed blob copied to another
  row or another account still opens.
- **Device Sentinel** needs a secret vault now (TOTP seeds, the agent CA key,
  the command-signing seed; later automation and probe credentials), with one
  ring per organisation, for its V1.

The owner decided on 2026-11-27 that the dxstorage code moves into **one shared
crate in Seiva** rather than being copied, the same path mutual TLS took
(ADR 0047): both products already depend on Seiva.

The forces:

- **The key-encryption key (KEK) never leaves its guardian.** In Vault it is the
  transit key; without Vault it exists only in the process's memory, read from a
  mounted file. Data keys are stored only wrapped.
- **A secret is bound to its context.** A blob moved to another record, another
  purpose or another organisation must not open. This is the improvement over
  dxstorage.
- **Rotation without downtime**, for both the KEK (in the guardian) and the data
  keys (in the application), with secrets re-sealed in the background.
- **Fail closed.** A corrupted ring, a key that did not unwrap, a guardian that
  is down at start-up: an error, never a fixed key or plaintext.
- **No storage opinion.** dxstorage keeps its ring in its cluster store, the
  Sentinel in Postgres. The crate must not pick.
- **DORM-free, and the default build stays HTTP-client-free.**

## Decision

Add a workspace crate **`seiva-kms`**.

**The guardian port, `KeyWrapper`** (dxstorage's `Wrapper` with its own error
type instead of `anyhow`): `provider`, `kek_id`, `generate` (a new data key and
its wrapped form), `unwrap`, `rewrap` (to the newest KEK version, `None` when
already current) and `health`. Errors are a `KmsError` whose `Unavailable`
variant (unreachable, 5xx, 429, sealed) is the only one worth retrying;
`Refused` (authentication, permission, unknown key) is final.

**Two adapters in V1, each behind a Cargo feature:**

- **`local`** (default): `LocalKek` reads a file with one base64 32-byte key per
  line. The first wraps; the others only unwrap. A wrapped data key is
  `local:<kek-fingerprint>:<base64>` (XChaCha20-Poly1305), so after a new key is
  put on top of the file, the next load re-wraps the ring with it and the old
  line can go.
- **`vault`**: `VaultTransit` over reqwest + rustls (ring provider), with token,
  token file (re-read at every call), **AppRole** and **Kubernetes** auth; an
  extra CA; a namespace; a `wait_ready(timeout)` start-up gate. A login token is
  kept for half its lease and replaced (one new login, one retry) when Vault
  answers 403. Redirects are not followed and the token header is marked
  sensitive. Health is `sys/health` (standbys accepted) plus
  `auth/token/lookup-self`.

AWS KMS, Google Cloud KMS and Azure Key Vault are ported from dxstorage later,
behind the same port.

**The ring.** `WrappedRing` is the stored form: a format version, the current
`WrappedKey` and the previous ones (id, provider, KEK name, wrapped form,
creation time), and `rotated_at`; it serialises to JSON and is validated on read
(known format, valid unique ids, nothing empty). `KeyRing` is the unwrapped form.
It is **pure** (no I/O, no clock): `seal(aad, plain)` and `open(aad, sealed)`,
`rotated`, `retired`, `with_rewrapped`, `needs_reseal`. It is built only when
every key of the ring is unwrapped; a missing key is an error, never a partial
ring. Keys are wiped from memory on drop and never printed.

**The sealed format: `sv1:<key-id>:<base64(nonce ‖ ciphertext ‖ tag)>`**, with
**XChaCha20-Poly1305**, a random 24-byte nonce, and **associated data**.
`Aad::new(org, purpose, record)` builds `<org>|<purpose>|<record>` (organisation
and purpose may not contain `|`, so the split is unambiguous); `Aad::raw` takes
any non-empty bytes. Empty associated data is refused: a secret is always bound
to something. A blob that does not authenticate returns `KmsError::Decrypt`,
which does not say whether the key, the associated data or the bytes were wrong.

**dxstorage compatibility, read-only.** `KeyRing::open_kms1` opens dxstorage's
`kms1:` blobs (AES-256-GCM, no associated data), and
`WrappedRing::from_dxstorage_json` reads its stored ring with its key ids, so
dxstorage can adopt the crate and re-seal progressively. `open` refuses `kms1:`
so that a caller never accepts an unbound blob where it expects a bound one.
`with_open_only` adds keys outside the ring that only open (dxstorage's static
keys).

**The caller's ports.** `RingStore` keeps one wrapped ring per *scope* (an
organisation, a deployment) with **compare-and-swap** writes (`load` returns the
ring and a version; `store` writes only if the version is unchanged). A ring
that does not parse is an error from `load`, never "absent", so a corrupted ring
is never replaced by a new one that would orphan every secret. `Reseal`
re-seals a scope's secrets with a given `KeyRing`. `MemoryRingStore` ships for
tests.

**The life cycle, `RingKeeper`**, stateless over a `KeyWrapper` and a
`RingStore`:

- `load(scope)`: create the ring on first use (one writer wins the race, the
  others read its ring), otherwise check it was made by this guardian and KEK,
  unwrap every key and re-wrap any key under an older KEK version (stored
  best-effort);
- `boot(scope, timeout)`: `load` retrying `Unavailable` with back-off (0.5 s
  doubling to 8 s) until the deadline, then the error: the service does not
  report ready;
- `rotate(scope, resealers)`: a new data key seals, the current one joins the
  previous, stored by compare-and-swap (`Conflict` if another replica got there
  first), then every resealer runs; their failures are reported, not fatal;
- `rotate_if_older` for a schedule;
- `retire(scope, after, resealers)`: once `after` has passed since the rotation,
  every resealer must succeed, then the previous keys are dropped.

Caching the `KeyRing` in memory, re-reading it on a notification or a timer, and
cross-replica locks belong to the caller.

## Alternatives considered

- **Copy dxstorage's code into the Sentinel.** Rejected by the owner: two
  lineages of the same security code drift, and fixes land in one only.
- **A separate repository for the crate.** Rejected: both users already depend
  on Seiva, and Seiva's CI gates (fmt, clippy, deny, docs) apply for free.
- **AES-256-GCM for the new format, as dxstorage does.** Rejected: a random
  12-byte nonce limits how many messages one key may seal before collisions
  become a risk. XChaCha20-Poly1305's 24-byte nonce removes that limit in
  practice, and it is pure Rust (RustCrypto) with no hardware dependency. The
  Sentinel already uses ChaCha20-Poly1305.
- **Accept `kms1:` in `open`.** Rejected: an attacker who can write a column
  could plant an unbound blob that opens regardless of the record. Reading
  `kms1:` is an explicit migration call.
- **Let the crate own storage (a Postgres table, a file).** Rejected: the two
  users store rings in entirely different places; compare-and-swap is the one
  property the crate needs, so it is the port.
- **Rotation state inside the crate (a background task, a cache).** Rejected for
  now: the notification mechanism (Postgres `NOTIFY`, dxstorage's peer
  broadcast) is the application's. `RingKeeper` stays stateless.
- **The Vault client over hyper directly.** Rejected: reqwest is what both users
  and `seiva-llm-reqwest` already use; keeping it behind the `vault` feature
  keeps the default build free of it.

## Consequences

- A project adds `seiva-kms` (with `vault` if needed), implements `RingStore`
  over its database and `Reseal` over its secret tables, and calls
  `RingKeeper::boot` before reporting ready (ADR 0012).
- Secrets sealed by the crate are bound to `org|purpose|record`; a project must
  pick the record id carefully (stable for the row's life) and keep it the same
  when re-sealing.
- The default build gains only RustCrypto's `chacha20poly1305` and `aes-gcm`
  (MIT/Apache) and `zeroize`. With `vault`, reqwest + rustls (ring). Both pass
  `cargo deny`.
- `cargo test --workspace` runs the ring, local and keeper tests. The `kms-vault`
  CI job runs the adapter's tests and the live tests against a
  `hashicorp/vault` service container (transit, a transit key rotation and the
  re-wrap, a data-key rotation and retirement, AppRole with a re-login).
- Not covered yet: the Kubernetes auth method and an HTTPS Vault with a private
  CA are implemented but tested only up to their configuration (no cluster or
  TLS Vault in CI); signing in the guardian (Vault transit Ed25519) for keys that
  should never leave it; the cloud KMSs.
- dxstorage keeps its `kms.rs` until its own slice adopts the crate (after its
  F15); its rings and `kms1:` blobs are readable as they are.

## Related

- ADR 0002: ports and in-memory test doubles (`MemoryRingStore` follows it).
- ADR 0012: readiness (`boot` failing closed keeps the service not ready).
- ADR 0036: MFA (TOTP seeds are the first secrets to move into a ring).
- ADR 0047: mutual TLS, the same "shared capability moves into Seiva" path.
