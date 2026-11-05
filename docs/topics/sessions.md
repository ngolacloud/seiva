# Sessions

Seiva services are **token-based** by default, not session-based — which is the
right default for an API.

## Prefer tokens

A request authenticates with a bearer token, verified per request into a
`Principal` ([Authentication](authentication.md)). There is no server-side session
store to keep, replicate or invalidate — the token carries the identity and the
service stays stateless, which is what lets it scale horizontally and deploy as
one binary with no sticky sessions.

## When you genuinely need session state

Some flows are stateful (a multi-step wizard, a shopping cart, a server-rendered
login). Model that state as **data** — a row your module owns, keyed by the
`Principal`'s subject — through the normal data layer, not a magic session object.
Then it is migrated, queried and authorized like any other data (including
object-level `Scoped<T>` checks), and it survives a restart.

For an opaque session *token* (rather than a JWT), the same `Authenticator` seam
applies: a `TokenAuthenticator`-style adapter can resolve an opaque token to a
`Principal` against your store.

> Scope: Seiva ships the token path (verified JWT/OIDC) as the default; a
> batteries-included cookie-session helper is not part of the framework today.

## Related

- [Authentication](authentication.md) · [Configuration](configuration.md)

---

*Part of the [Seiva documentation](../index.md).*
