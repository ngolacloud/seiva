# Topic guides ("Using Seiva")

These guides are **understanding-oriented**: each one discusses a layer or concern
in depth, explaining not just *how* but *why*. They are the Seiva equivalent of
Django's "Using Django" topic guides.

## The layers

- **[The data layer (DORM)](data-layer/index.md)** — models, queries, migrations,
  transactions, multiple databases. (Django's *model layer*.)
- **[The transport layer](transport/index.md)** — facades, controllers, routing,
  requests/responses, the middleware stack, composition. (Django's *view layer*.)
- **[Validation and DTOs](validation.md)** — validated input and RFC 9457 errors.
  (Django's *forms*.)
- **[API documents](api-documents.md)** — OpenAPI, the docs UI and proto. (Where
  Seiva, being API-first, differs from Django's *template layer*.)

## Identity, security, correctness

- [Authentication](authentication.md) — one verified `Principal`, OIDC/JWT, deny-by-default
- [Authorization](authorization.md) — object-level, fail-closed `Scoped<T>` (BOLA)
- [Security](security.md) — the secure-by-default posture (headers, rate limit, OWASP API Top 10)
- [Testing](testing.md) — fakes, controller tests, architecture fitness functions

## Operating a service

- [Configuration](configuration.md) — typed 12-factor config and the AppEnv guard
- [Observability](observability.md) — structured logs, correlation, tracing
- [Performance & cloud-native](performance.md) — single binary, distroless, probes

## Cross-cutting tools

- [Messaging & events](messaging-and-events.md) — CloudEvents, publishers, the outbox
- [CQRS dispatch](cqrs.md) — command/query mediation with typed middleware
- [Background workers](background-workers.md) — work beyond the request
- [Caching](caching.md) · [Pagination](pagination.md) · [Sending email](sending-email.md)
  · [Sessions](sessions.md) · [Serialization](serialization.md)

---

*Part of the [Seiva documentation](../index.md).*
