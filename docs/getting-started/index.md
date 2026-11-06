# Getting started

New to Seiva? Start here. These pages are **learning-oriented**: follow them in
order and you will go from an empty checkout to a running, validated,
authenticated and packaged service.

## First steps

1. [Seiva at a glance](seiva-at-a-glance.md) — what Seiva is, the one big idea,
   and what you get out of the box.
2. [Quick install guide](quick-install.md) — install the `seiva` CLI and create a
   project. (The fuller version is the [quickstart](../quickstart.md).)
3. [Coming from a dynamic framework](../first-day.md) — map each habit you bring
   to its Seiva equivalent, honestly marking what exists today and what is in progress.

## The tutorial: building a shop

A guided build of a five-module **e-commerce shop** (catalog · inventory · orders ·
payments · notifications) — one concern per part, from an empty checkout to a
backend that runs as a monolith *and* as microservices, plus a browser storefront.
The modules cooperate through **events**, not calls.

1. [A modular monolith](shop/part-1-the-modular-monolith.md) — scaffold the shop and its five modules
2. [Modules own their data](shop/part-2-modules-own-their-data.md) — the five contracts and their permissions
3. [Events between modules](shop/part-3-events-between-modules.md) — the order flow, wired by events
4. [The data layer](shop/part-4-the-data-layer.md) — real tables per module with DORM
5. [Authorization across modules](shop/part-5-authorization.md) — who may do what, enforced by the contract
6. [Testing the shop](shop/part-6-testing.md) — fakes, the event flow, fitness functions
7. [Monolith to microservices](shop/part-7-monolith-to-microservices.md) — split `payments` + `notifications`, the same code
8. [A browser frontend with dxjs](shop/part-8-a-dxjs-frontend.md) — a CSP-safe storefront consuming the shop API

## Where to next

[What to read next](what-to-read-next.md) — the topic guides, the reference, and
the design internals.

---

*Part of the [Seiva documentation](../index.md).*
