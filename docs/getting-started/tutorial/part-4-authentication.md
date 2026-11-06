# Part 4: Authentication and permissions

The polls facade declares a permission on every method — `polls.manage` to create
questions and choices, `polls.vote` to vote, `polls.read` to list and read
results. In this part we see what those guards mean and how to admit real callers.
(Where other frameworks bolt auth on per route, Seiva's permission is declared on
the contract and enforced by the transport.)

## The routes are guarded by default

With no authenticator wired, the framework is **deny-by-default**: a guarded route
answers `403`, because no caller is ever authenticated.

```console
$ curl -s -o /dev/null -w '%{http_code}\n' -XPOST localhost:8080/v1/polls/vote \
    -H 'content-type: application/json' -d '{"choice_id":1}'
403
```

A *public* route (a facade method with no `#[requires]`) serves to anyone. If you
wanted the index of questions to be world-readable, you would drop `#[requires]`
from `list`.

## Admit callers with an authenticator

Point the server at your OIDC provider's JWKS; it builds a `JwtAuthenticator` and
mounts it ahead of every route's permission check:

```console
$ export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
$ export SEIVA_OIDC_ISSUER=https://id.example.com/
$ export SEIVA_OIDC_AUDIENCE=polls-site
$ seiva start
```

Now the behaviour is:

| Request | Result |
| --- | --- |
| no bearer | `anonymous` — public routes serve, guarded routes `403` |
| invalid bearer | `401` + `WWW-Authenticate` |
| valid bearer | verified `Principal`; the route's `polls.*` permission is checked |

So a token carrying `polls.vote` may vote but not create questions; a token with
`polls.manage` may do both. Tokens are verified cryptographically (RS256/ES256
only, never `alg:none`; `iss`/`aud`/`exp` validated). Permissions come from a
configurable claim. See [Authentication](../../topics/authentication.md).

## Object-level authorization

Function-level permission (`#[requires]`) says *who may call*; object-level
authorization says *which rows they may touch* — for a multi-tenant polls service,
which tenant's questions a caller can see. Seiva's `Scoped<T>` makes
cross-tenant/cross-owner access a compile-time concern, fail-closed — see
[Authorization](../../topics/authorization.md) and [Security](../../topics/security.md).

## Next

[Part 5: Testing your module](part-5-testing.md).

---

*Part of the [Seiva documentation](../../index.md).*
