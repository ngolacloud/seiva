# seiva-kms

Envelope encryption for [Delonix Seiva](https://github.com/ngolacloud/seiva)
(ADR 0049): data keys wrapped by a key-encryption key that never leaves its
guardian, a key ring that seals secrets bound to their context, and rotation
with re-sealing.

```rust,ignore
use std::sync::Arc;
use std::time::Duration;
use seiva_kms::{Aad, LocalKek, RingKeeper};

let kek = Arc::new(LocalKek::from_file("/run/secrets/kek")?);
let keeper = RingKeeper::new(kek, Arc::new(my_postgres_ring_store));
let ring = keeper.boot("org-42", Duration::from_secs(60)).await?; // fails closed

let aad = Aad::new("org-42", "identity.totp", "user-7")?;
let sealed = ring.seal(&aad, b"JBSWY3DPEHPK3PXP")?;      // "sv1:local-…:…"
let plain = ring.open(&aad, &sealed)?;                    // wiped on drop
```

- **`KeyWrapper`** — the guardian port. `LocalKek` (feature `local`, default):
  one base64 32-byte key per line, the first wraps, the rest only unwrap.
  `VaultTransit` (feature `vault`): Vault/OpenBao transit with token, token file,
  AppRole or Kubernetes auth, an extra CA and `wait_ready(timeout)`.
- **`KeyRing`** — pure: `seal(aad, plain)` → `sv1:<id>:<base64>`
  (XChaCha20-Poly1305 with associated data), `open(aad, sealed)`, and
  `open_kms1` for dxstorage's `kms1:` blobs (read only).
- **`RingStore` / `Reseal`** — you keep the wrapped ring (compare-and-swap) and
  re-seal your secrets; `MemoryRingStore` for tests.
- **`RingKeeper`** — `load` / `boot` (create, unwrap, re-wrap after a KEK
  rotation), `rotate`, `rotate_if_older`, `retire`.

The live Vault tests run with `SEIVA_KMS_TEST_VAULT_ADDR` and
`SEIVA_KMS_TEST_VAULT_TOKEN` set (see `tests/vault_live.rs`).

License: MIT OR Apache-2.0.
