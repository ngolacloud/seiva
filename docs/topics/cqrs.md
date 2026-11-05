# CQRS dispatch

Seiva offers a statically-wired command/query mediator (`seiva-cqrs`) for the
application layer — with typed middleware — **when a module's complexity earns
it**. It is a tool, not a mandate.

## What it is

A command/query is a typed message; a handler processes it; a mediator dispatches
message → handler. Cross-cutting concerns (instrumentation, query caching) are
**typed middleware** in the dispatch pipeline, wired at compile time — no runtime
reflection, no service locator. It is data-layer-free: the dispatch is about the
application layer, independent of how data is stored.

```rust
// a command + its handler, dispatched through the mediator with middleware
let dto = mediator.send(CreateRecord { title }).await?;
```

## When to use it — and when not

Use CQRS when a module has **many** use cases, or read and write paths that differ
enough to want separate models or caching on reads. For a module that is plain
CRUD over one model, the facade method calling the application service directly is
simpler and clearer — forcing CQRS on trivial CRUD is an anti-pattern the
`seiva-pattern-fitness` catalogue flags.

Seiva does not impose CQRS: a module starts with a direct facade→service call, and
adopts the mediator only if its complexity justifies it.

## Middleware

The pipeline's middleware is typed and composable — e.g. an instrumentation layer
that spans every command, or a query-cache layer keyed by the query value. You add
exactly the cross-cutting behaviour the module needs, checked by the compiler.

## Related

- [Facades](transport/facades.md) · [The architecture](../architecture.md)
- [Background workers](background-workers.md)

---

*Part of the [Seiva documentation](../index.md).*
