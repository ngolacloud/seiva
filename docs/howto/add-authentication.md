# How to add authentication

Guarded routes already deny by default. To admit real callers, wire an OIDC/JWT
authenticator.

1. **Obtain your provider's JWKS** (the public keys that verify its tokens) and
   save it to a file the service can read.

2. **Set the OIDC environment** (a partial set fails the boot, by design):

   ```console
   export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
   export SEIVA_OIDC_ISSUER=https://id.example.com/
   export SEIVA_OIDC_AUDIENCE=my-api
   # optional: SEIVA_OIDC_TENANT_CLAIM, SEIVA_OIDC_PERMISSIONS_CLAIM
   ```

   `seiva-server` builds a `JwtAuthenticator` from the JWKS and mounts it ahead of
   every route's permission check.

3. **Guard your facade methods** with the permissions callers must hold:

   ```rust
   #[requires("billing.charge")]
   async fn charge(&self, cmd: Charge) -> Result<ReceiptDto, BillingError>;
   ```

Behaviour: no bearer → anonymous (public routes serve, guarded `403`); invalid
bearer → `401`; valid bearer → verified `Principal`, permission checked. Tokens
are RS256/ES256 only, `iss`/`aud`/`exp` validated.

For per-row (object-level) checks, use `Scoped<T>` — see [Authorization](../topics/authorization.md).

## Related

- [Authentication](../topics/authentication.md) · [Security](../topics/security.md) · [Settings](../reference/settings.md)

---

*Part of the [Seiva documentation](../index.md).*
