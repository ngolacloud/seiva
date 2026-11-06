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

## The tutorial

A guided build of a **polls** app (questions, choices, voting, results) — one
concern per part, learning-oriented from an empty checkout to a split-out
microservice:

1. [Your first module](tutorial/part-1-your-first-module.md) — scaffold the polls module and meet the five layers
2. [Validation and DTOs](tutorial/part-2-validation-and-dtos.md) — `Valid<T>`, RFC 9457, `WireSchema`
3. [Persisting with the data layer](tutorial/part-3-the-data-layer.md) — swap in DORM
4. [Authentication and permissions](tutorial/part-4-authentication.md) — verified `Principal`, `#[requires]`
5. [Testing your module](tutorial/part-5-testing.md) — fakes, controller tests, fitness functions
6. [Packaging and deployment](tutorial/part-6-packaging-and-deploy.md) — distroless, non-root, `/health/ready`
7. [From monolith to microservice](tutorial/part-7-monolith-to-microservice.md) — `seiva module split`, the same code standalone

## Where to next

[What to read next](what-to-read-next.md) — the topic guides, the reference, and
the design internals.

---

*Part of the [Seiva documentation](../index.md).*
