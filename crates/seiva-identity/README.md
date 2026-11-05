# seiva-identity

The caller and how a transport authenticates it, for the Delonix Seiva
framework. Transport-free (no HTTP types) so every transport — `seiva-http`,
`seiva-mcp`, a future gRPC — shares one identity model.

- [`Principal`] — the authenticated caller: a subject, the tenant it is scoped
  to (if any), and the permissions it holds.
- [`Authenticator`] — turns a bearer token into a `Principal`, **denying by
  default**: an empty or unrecognised bearer is a rejection, never a silent pass.
- [`TokenAuthenticator`] — a static token → principal map, the dev/test impl.
- [`JwtAuthenticator`] — verifies an OIDC/JWT against an injected JWKS (RS256 /
  ES256 only, validating `iss`/`aud`/`exp`/`nbf`) and maps its claims to a
  `Principal`. It never fetches keys itself, so it stays network-free and
  deterministically testable.

See `docs/adr/0006-identity-and-authentication.md`.
