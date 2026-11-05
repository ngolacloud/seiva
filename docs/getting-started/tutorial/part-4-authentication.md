# Part 4: Authentication and permissions

> **Status: planned.** This page is a planned part of the documentation structure; the outline below is the intended content.

Protect a route with a verified Principal.

## On this page

- `#[requires("notes.create")]` on a facade method
- Wiring an `Authenticator` (OIDC/JWT) in the composition root
- deny-by-default: anonymous vs 401 vs 403
- Object-level authorization with `Scoped<T>`

## Related

- [Authentication (topic guide)](../../topics/authentication.md)
- [Identity (ADR 0006)](../../adr/0006-identity-and-authentication.md)

---

*Part of the [Seiva documentation](../../index.md), organised like the Django
docs (tutorials · topic guides · reference · how-to).*
