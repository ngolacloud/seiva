# Part 7: Monolith to microservices

The shop has shipped as one binary. Now two concerns need to move out on their
own: **payments** must run in an isolated deployment (its own secrets, its own
blast radius), and **notifications** does slow, bursty work that should scale
independently. In most stacks that is a rewrite. In Seiva it is a composition
change — the module code does not move.

## Split a module into its own service

```console
$ seiva module split payments
$ seiva module split notifications
```

`seiva module split` promotes a module to a standalone service: it gets its own
composition root (`apps/payments-api`), its own image, and its own deployment,
while the **exact same** `modules/payments/` crates compile into it. The remaining
monolith (`catalog`, `inventory`, `orders`) keeps running as one binary.

Nothing inside `modules/payments/` changed — not the facade, not the domain rule,
not the use case. That is the whole point of coding the application layer against
ports and keeping modules independent (Parts 2–3): *where* a module runs is a
deployment decision, taken last.

## The events now cross the network

In the monolith, `orders.placed` reached the payment reaction over an in-process
bus. With `payments` split out, the *same* event crosses a **broker topic**
instead — and because the publish already happened inside the order's transaction
and landed in the outbox (Part 4), the relay worker forwards it to the broker
exactly once. The reaction code is identical; only the transport under it changed.

```text
monolith:      orders ──(in-process bus)──▶ payments.charge
microservices: orders ──(outbox → broker topic)──▶ payments service ──▶ payments.charge
```

The `payments.charge` permission (Part 5) now also does real work at the service
boundary: the payment service accepts the reaction only from a caller holding that
capability, so a split does not widen the attack surface.

## When to split — and when not to

Start as a monolith. One deploy, one transaction, one place to look — until a
module has a *reason* to move: isolation (payments), independent scaling
(notifications), a separate team, a different runtime. Seiva lets you defer that
decision because the boundary is real from day one; splitting is cheap precisely
because you did not have to predict it.

## You built a shop

From `seiva project new` to a five-module shop that runs as a modular monolith and
then as a fleet of services — the same code, a deployment knob. Where next:

- The [topic guides](../../topics/index.md) for depth on each layer.
- The [DORM section](../../dorm/index.md) and the [dx section](../../dx/index.md).
- The [competitive landscape](../../competitive-landscape.md) for where Seiva
  stands against other stacks.

---

*Part of the [Seiva documentation](../../index.md).*
