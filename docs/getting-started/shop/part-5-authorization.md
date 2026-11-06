# Part 5: Authorization across modules

Every method in the shop declares the permission a caller must hold. With five
modules, those permissions become a small capability map — and because each is
declared on the contract, the transport enforces it the same way everywhere, deny
by default.

## The shop's permissions

| Module | Permission | Guards |
| --- | --- | --- |
| catalog | `catalog.manage` · `catalog.read` | add products · browse |
| inventory | `inventory.manage` · `inventory.read` | receive/reserve stock · check availability |
| orders | `orders.manage` · `orders.read` | place orders · view them |
| payments | `payments.charge` · `payments.read` | charge · view a payment |
| notifications | `notifications.send` · `notifications.read` | send · list |

A token carrying `catalog.read` can browse the catalogue but not add a product; a
back-office token with `catalog.manage` can do both. The `payments.charge`
permission is deliberately narrow — the charge reaction in the order flow runs
under a service identity that holds exactly that, nothing else.

## Deny by default

With no authenticator wired, every guarded route answers `403` — no caller is ever
authenticated, so nothing is permitted. Point the server at your OIDC provider's
JWKS and it verifies bearer tokens (RS256/ES256, `iss`/`aud`/`exp` checked) and
reads the permissions from a configurable claim:

```console
$ export SEIVA_OIDC_JWKS_FILE=/etc/seiva/jwks.json
$ export SEIVA_OIDC_ISSUER=https://id.example.com/
$ export SEIVA_OIDC_AUDIENCE=shop
$ seiva start
```

| Request | Result |
| --- | --- |
| no bearer | `anonymous` — guarded routes `403` |
| invalid bearer | `401` + `WWW-Authenticate` |
| valid bearer | the route's `<module>.<action>` permission is checked |

## Object-level authorization

Function-level permission says *who may call*; object-level authorization says
*which rows they may touch* — for a multi-tenant shop, which merchant's orders a
caller can see. Seiva's `Scoped<T>` makes cross-tenant access a compile-time,
fail-closed concern. See [Authorization](../../topics/authorization.md) and
[Security](../../topics/security.md).

## Next

[Part 6: CQRS — separating reads from writes](part-6-cqrs.md).

---

*Part of the [Seiva documentation](../../index.md).*
