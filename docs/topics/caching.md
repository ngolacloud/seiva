# Caching

Cache deliberately, at the layer that owns the invalidation.

## Query caching (application layer)

The CQRS mediator ([CQRS](cqrs.md)) supports **typed query-cache middleware**:
a query handler's result is cached keyed by the query value, with the middleware
owning the lifetime. This is the first-class caching seam — it caches *use-case
reads*, where the module knows when a result is stale (it wrote the change), so
invalidation is correct rather than guessed.

## HTTP caching (transport)

For public, cacheable `GET`s you can set HTTP caching headers (`Cache-Control`,
`ETag`) on the response so clients and proxies cache them. Use this only for
responses that are genuinely safe to cache; the framework's defaults do not cache.

## What not to do

Do not cache behind the data layer blindly (a global read-through cache) — that is
where stale-data bugs live, because the cache does not know when a write
invalidated an entry. Cache where the write happens (the application layer), or at
the edge for immutable/public responses.

> Scope: the CQRS query-cache middleware is the supported path today; richer
> cache backends and HTTP-cache helpers are an area that grows with real use.

## Related

- [CQRS dispatch](cqrs.md) · [Performance & cloud-native](performance.md)

---

*Part of the [Seiva documentation](../index.md).*
