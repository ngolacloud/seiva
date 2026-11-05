# Configuration (12-factor)

A Seiva service reads its configuration from the environment at startup — one
typed `ServerConfig`, validated once, with a production-safety guard. No config
file, no globals.

## `ServerConfig::from_process_env()`

The composition root calls it first; on an invalid configuration it logs and exits
non-zero rather than starting half-configured:

```rust
let config = match ServerConfig::from_process_env() {
    Ok(config) => config,
    Err(error) => { tracing::error!(%error, "invalid configuration"); return ExitCode::FAILURE; }
};
```

Core variables:

- **`SEIVA_HTTP_ADDR`** — the listen address (e.g. `0.0.0.0:8080`).
- **`APP_ENV`** — the environment (`development` / `production`). Drives the
  `AppEnv` production-safety guard: production refuses insecure defaults that are
  fine in development.

## Authentication (OIDC/JWT)

The server builds a `JwtAuthenticator` when the OIDC variables are set; a partial
set fails the boot (fail-closed):

- **`SEIVA_OIDC_JWKS_FILE`** — the JWKS to verify tokens against;
- **`SEIVA_OIDC_ISSUER`**, **`SEIVA_OIDC_AUDIENCE`** — validated claims;
- optional claim-name overrides for tenant and permissions.

See [Authentication](authentication.md).

## Data layer

The DORM data layer reads its DSN from **`DELONIX_ORM_DSN`** (used by
`seiva db dev/migrate` and the generated `data/dorm.schema`'s datasource). See
[the data layer](data-layer/index.md) and [multiple databases](data-layer/multiple-databases.md).

## The twelve-factor posture

Configuration is environment, not code: the same binary runs in dev and prod with
different env. The generated image sets `APP_ENV=production` and
`SEIVA_HTTP_ADDR=0.0.0.0:8080`, and the platform injects secrets — nothing is
baked in. The full variable list is in the [settings reference](../reference/settings.md).

## Related

- [The application and composition root](transport/application-and-composition.md)
- [Settings reference](../reference/settings.md) · [Performance & cloud-native](performance.md)

---

*Part of the [Seiva documentation](../index.md).*
