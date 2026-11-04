# seiva-auth

Credential primitives for the [Delonix Seiva](https://github.com/ngolacloud/seiva)
framework, moved out of DORM's CLI accounts (the brief's Parte DORM-A). It is
**DORM-free** and **data-layer-free**.

- `hash_password` / `verify_password` — Argon2id hashing producing and checking
  a PHC string (`$argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>`), with a fresh
  random salt per hash.
- `prompt_password` — a thin no-echo console prompt.
- `Permission` / `PermissionSet` — a minimal named-permission value type and a
  set with membership checks.

```rust
use seiva_auth::{hash_password, verify_password};

let phc = hash_password("correct horse battery staple")?;
assert!(phc.starts_with("$argon2id$"));
assert!(verify_password("correct horse battery staple", &phc)?);
# Ok::<(), seiva_auth::AuthError>(())
```

Licensed under either of MIT or Apache-2.0 at your option.
