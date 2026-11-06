# Frequently asked questions

### Is Seiva a web framework or an ORM?

Seiva is the **framework** — the service, distribution and domain layers around a
data source. The data source by default is **DORM** (the Delonix ORM), an
independent product, wired in only through the optional `dorm` feature. The
framework core never depends on DORM.

### What does Seiva give me beyond routing and validation?

A thin service layer stops at routing + validation + auto-OpenAPI (no ORM, no
migrations, no auth system, no scaffolding, no admin). Seiva keeps the
auto-OpenAPI + validation developer experience and adds the batteries — an
optional data layer, a verified-JWT auth system, project/module scaffolding,
security-by-default — as a **compiled, no-GC, single binary**. See
[Seiva at a glance](../getting-started/seiva-at-a-glance.md) and the
[competitive landscape](../competitive-landscape.md).

### Do I have to use DORM?

No. Every module's application layer codes against trait-only *ports*. The default
adapters are in-memory (DORM-free), and you can back a repository with any external
data source. DORM is the batteries-included default, not a lock-in.

### Why Rust, when Prisma just moved *off* Rust?

Prisma moved its *client* off Rust because the **Rust↔JavaScript FFI boundary** was
the bottleneck — not Rust itself. A native-Rust consumer (DORM, Seiva) has no such
boundary: no GC, no FFI, compile-time safety to the binary, single-binary deploy.
See [the data layer](../topics/data-layer/index.md).

### Is there a built-in admin console?

Not yet — a full auto-CRUD admin is a roadmap item. It is one of the gaps called
out in the [competitive landscape](../competitive-landscape.md).

### What works today vs what is planned?

Seiva is at founding stage. The [engineering log](../progress.md) and the
[ADRs](../internals/adrs.md) track capabilities honestly; pages marked "planned" in
these docs carry the intended outline.

### Where do I report issues / contribute?

See [Contributing](../internals/contributing.md). An architectural change lands in
an [ADR](../internals/adrs.md) first.

---

*Part of the [Seiva documentation](../index.md).*
