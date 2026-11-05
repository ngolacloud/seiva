# Settings and environment

Seiva is configured entirely through the environment (12-factor). This is the full
list of variables the framework and the CLI read.

## The service (`ServerConfig`, read at startup)

| Variable | Meaning |
| --- | --- |
| `SEIVA_HTTP_ADDR` | The listen address, e.g. `0.0.0.0:8080`. |
| `APP_ENV` | `development` or `production`; drives the `AppEnv` production-safety guard (production refuses dev-only defaults). |

An invalid or incomplete configuration fails the boot (the service logs and exits
non-zero) rather than starting half-configured. See [Configuration](../topics/configuration.md).

## Authentication (OIDC/JWT)

The server builds a `JwtAuthenticator` when these are set; a partial set fails the
boot (fail-closed).

| Variable | Meaning |
| --- | --- |
| `SEIVA_OIDC_JWKS_FILE` | Path to the JWKS used to verify tokens. |
| `SEIVA_OIDC_ISSUER` | Expected `iss`. |
| `SEIVA_OIDC_AUDIENCE` | Expected `aud`. |
| `SEIVA_OIDC_TENANT_CLAIM` | Claim holding the tenant (optional; default `tenant`). |
| `SEIVA_OIDC_PERMISSIONS_CLAIM` | Claim holding permissions (optional; default `permissions`, with space-delimited `scope` supported). |

See [Authentication](../topics/authentication.md).

## Data layer (DORM)

| Variable | Meaning |
| --- | --- |
| `DELONIX_ORM_DSN` | The database DSN the data layer connects to — a libpq string for PostgreSQL/CockroachDB (`host=… dbname=…`) or a URL for MySQL (`mysql://…`). Read by `seiva db dev`/`migrate` and the generated `data/dorm.schema` datasource. |

See [the data layer](../topics/data-layer/index.md) and [multiple databases](../topics/data-layer/multiple-databases.md).

## CLI / tooling

| Variable | Meaning |
| --- | --- |
| `SEIVA_EMIT` | Set to `openapi` or `proto` to make the composition root print that document and exit instead of serving (how `seiva openapi`/`proto` document a project). |
| `SEIVA_FRAMEWORK_DIR` | Overrides the framework checkout a generated project's interim `[patch.crates-io]` points at (until the crates are published). |
| `SEIVA_DORM_BIN` | The `dorm` CLI binary `seiva db` delegates to (else `dorm` on `PATH`). |
| `SEIVA_DORM_DIR` | A local DORM checkout for the data layer's interim patch (else the sibling `../DORM`; absent → resolve DORM from crates.io). |

## Related

- [Configuration (topic guide)](../topics/configuration.md) · [The `seiva` CLI](cli.md)

---

*Part of the [Seiva documentation](../index.md).*
