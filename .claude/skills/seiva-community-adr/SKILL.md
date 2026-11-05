---
name: seiva-community-adr
description: Ensure any new architecture decision lands in a legible ADR (context, alternatives, consequences) before it is implemented. Use when a change introduces or alters an architectural pattern, boundary, or public contract.
---

# Seiva community ADR

Seiva aims to be a public reference framework, so an outside reader must be able
to understand *why* each architectural choice was made. A new decision that is
not written down is a decision the community cannot adopt or challenge.

## When this applies

A change needs an ADR when it: adds or removes a crate/layer; changes a layer
dependency rule; alters a public contract (`FacadeDescriptor`, a port trait, the
wire shape); introduces a cross-cutting pattern (a new middleware, a messaging
backend, an auth strategy); or changes the data-layer boundary.

A pure bug fix, a doc edit, or a mechanical refactor does **not** need one.

## The shape (match `docs/adr/0001`–`0004`)

Numbered `docs/adr/NNNN-kebab-title.md`, each with:

1. **Context** — the forces and constraints, in plain language an outsider gets.
2. **Decision** — what we chose, stated as one clear sentence then the detail.
3. **Alternatives considered** — the real options and why each was not taken
   (this is what makes it legible, not a rubber stamp).
4. **Consequences** — what becomes easier, what becomes harder, what debt we
   accept (dated), and how it is enforced (a fitness function? a CI gate?).

## Run

1. Decide whether the change crosses the "needs an ADR" bar above.
2. If yes and no ADR exists, draft one with the four sections and the next free
   number; link it from the PR and from `docs/progress.md`.
3. Verify it is readable without the codebase open: an outside Rust developer
   should understand the trade-off from the ADR alone.

## Output

Either "no ADR needed (reason)" or the drafted ADR path + a one-line legibility
verdict.
