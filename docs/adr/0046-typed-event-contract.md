# ADR 0046 — A domain event is a published contract: a typed, versioned, schema-bearing event instead of an ad-hoc JSON blob

- Status: Proposed
- Date: 2026-11-15
- Deciders: NgolaCloud
- Follows: ADR 0007 (WireSchema contract fidelity), ADR 0021 (in-process event bus and choreography), ADR 0003 (module layering)

## Context

Seiva's central promise is that **modules cooperate through events, not calls**
(CLAUDE.md, ADR 0021). The synchronous half of that cooperation — the facade — is
a first-class contract: a `#[seiva::facade]` trait whose DTOs derive `WireSchema`
and are published to OpenAPI, versioned, permission-guarded, the single source of
truth (ADR 0007). A consumer of a facade compiles against a typed, discoverable
contract.

The asynchronous half — the event — has **no contract at all**. The scaffold
teaches a developer to publish like this (`scaffold.rs` `application_lib`):

```rust
let event = serde_json::to_vec(&serde_json::json!({
    "id": record.id.to_string(),
    "title": record.title.clone(),
}))?;
self.events.publish("notes.record.created", &event).await
```

The topic is a bare string; the payload is an anonymous JSON object built by hand
inside the use case. On the consuming side (`scaffold.rs` `event_main_rs`) a
subscriber receives a `CloudEvent` and reads `event.ty` as a string, then must
`serde_json::from_slice` the body into *some* shape it hopes the producer emits.
ADR 0021 itself names this: the dispatcher "deserialises each CloudEvent into the
subscriber's concrete type" — but there is no producer-side type that guarantees
the bytes match that concrete type.

So the channel the framework declares as the **primary** coupling between modules
is the only one with no published language. The forces this creates:

- A producing module changes `title` to `name`; nothing fails to compile. Every
  consumer breaks at runtime, on the next event, in a different process — exactly
  the failure mode the typed facade was built to prevent.
- A consumer has nothing to generate documentation, a schema registry entry, or a
  mock from. The event is undiscoverable: `seiva openapi` describes every route but
  no event.
- The scaffold is the most-copied code in the framework (the architecture review's
  recurring theme). It teaches `serde_json::json!` ad-hoc envelopes as the norm, so
  every real module reinvents an unversioned, unvalidated event shape.

This is the gap finding **A2** (Alto) in `docs/audits/2026-11-architecture-review.md`.
It is a Published-Language / Context-Mapping gap (Fowler) and a domain-model gap
(Evans — the domain event is a first-class concept, not a dictionary literal).

The constraint the solution must respect: ADR 0021 fixed the **transport** of
events (the transactional outbox, CloudEvents 1.0 on the wire, at-least-once with
inbox dedup, the same subscriber code in monolith and after a split). ADR 0046
must add a **contract** over that transport **without** changing the transport or
the layering rule (ADR 0003): the event type must live where a facade DTO lives
(the contract layer), be transport-neutral, and not make a consumer import the
producer's internal crates.

## Decision

**A domain event is declared as a typed, versioned, schema-bearing contract in the
publishing module's `facade` crate — the same layer, and the same contract
discipline, as its request/response DTOs.** The event is the Published Language of
the module's asynchronous boundary.

1. **A typed event payload with `WireSchema`.** The producing module declares the
   event body as a struct in its `facade` crate, deriving the same
   `serde` + `WireSchema` as a response DTO, so its fields are discoverable and
   publishable exactly like a facade DTO (ADR 0007):

   ```rust
   // in <module>-facade
   #[derive(Debug, Clone, Serialize, Deserialize, WireSchema)]
   pub struct RecordCreated {
       pub id: String,
       pub title: String,
   }
   ```

   The type lives in `facade` because it is part of the module's public contract,
   not its domain internals — a consumer may name it without reaching the
   producer's `domain`/`application`, preserving the ADR 0003 layering and the
   independence `module split` relies on.

2. **A versioned, declared topic.** The event declares its type string **once**,
   next to the payload, with an explicit version suffix — `notes.record.created.v1`
   — instead of a free string typed at the `publish` call site. A macro or a const
   on the type is the single source of that string, so producer and the CloudEvent
   `ty` cannot drift. The version is part of the contract: a breaking change to the
   payload is `…​.v2`, published alongside `…​.v1` until consumers migrate — the
   asynchronous twin of facade versioning.

3. **A link-time event registry, mirroring the facade registry.** Each event type
   is registered at link time (as facades and seeds already are —
   `registered_facades`, `registered_seeds`), so `seiva openapi` (or a new
   `seiva events`) can emit the catalogue of events a binary produces, and a fitness
   test can assert the generated scaffold's emitted bytes match the registered
   schema. Events become as discoverable as routes.

4. **Publication and transport are unchanged (ADR 0021 holds).** The use case still
   publishes **inside** its `UnitOfWork::transaction` through the `EventPublisher`
   port, still into the transactional outbox, still CloudEvents 1.0 on the wire,
   still at-least-once with inbox dedup. The only change is that the payload it
   serialises is a typed value (`serde_json::to_vec(&RecordCreated { … })`) under a
   declared topic, not a hand-built `json!` literal under a string. The subscriber's
   `from_slice::<RecordCreated>` now has a producer-side guarantee behind it.

5. **The scaffold teaches the typed shape.** `scaffold.rs` generates the
   `RecordCreated` event type in the facade, publishes it typed in the application,
   and (for `--kind event`) deserialises it typed in the subscriber — so the
   most-copied code models the contract, not the ad-hoc blob. This is what closes
   A2: the fitness `generated_event_matches_its_registered_schema` goes green.

## Alternatives considered

- **Leave the event as ad-hoc JSON, document a convention.** Rejected: it is the
  status quo A2 names. A convention that the compiler does not enforce is reinvented
  (and broken) per module; the framework already rejected this for the synchronous
  contract (ADR 0007) and should not hold a double standard for the channel it calls
  primary.
- **Put the event type in `domain` (the event is a domain concept — Evans).** The
  event *originates* in the domain, but placing the published type in `domain` would
  force a consumer to depend on the producer's `domain` to name it, violating ADR
  0003 and the independence `module split` needs. We keep the **domain event** as a
  domain concept internally and expose its **published** form as a facade-layer
  contract — the same split the facade already makes between a domain entity and its
  wire DTO.
- **A central shared `events` crate every module imports.** Rejected: it recreates
  the compile-time coupling between modules that events exist to avoid — a consumer
  would transitively couple to every producer. Each event is owned by its producing
  module's facade, discovered at link time, never by cross-module import.
- **Generate the event schema from the topic string at runtime (reflection).**
  Rejected: Seiva's contract is compile-time with no reflection (CLAUDE.md); the
  event catalogue must be known statically like the OpenAPI document.
- **A full external schema registry (Avro/Protobuf registry).** Deferred, not
  rejected: the CloudEvents wire envelope could later carry a `dataschema` pointing
  at a published JSON Schema derived from `WireSchema`. ADR 0046 establishes the
  in-repo typed contract and the link-time catalogue first; wiring a remote registry
  is a later, narrower step.

## Consequences

- **Easier:** a consumer compiles against a typed event; a breaking payload change
  is a visible `.v2`, not a silent runtime break; events are discoverable
  (`seiva events` / the catalogue) like routes are; the scaffold teaches the
  contract, so new modules inherit it instead of reinventing a blob.
- **Harder / debt (dated 2026-11-15):** a link-time event registry and the
  `WireSchema`-of-event plumbing are new code (close kin to the facade registry, but
  new). Existing hand-written example modules and any ad-hoc `json!` events in
  `apps/*` must migrate to the typed shape — a tracked, mechanical change, like the
  F4 example migration. Topic versioning adds a `.vN` discipline producers must
  follow; the framework should lint a topic string that carries no version.
- **Enforcement (the closing fitness functions for A2):**
  `generated_event_matches_its_registered_schema` — the bytes the scaffolded
  application publishes deserialise into the registered `WireSchema` event type;
  `event_catalogue_lists_the_module_event` — a binary's registered events are
  emitted by `seiva events`/the catalogue; and the existing choreography e2e keeps
  proving one effect per event, now over a typed payload.

## Related

- [ADR 0003](0003-module-layering-and-facade-impl.md) · [ADR 0007](0007-wire-schema-contract-fidelity.md) · [ADR 0011](0011-durable-idempotency.md) · [ADR 0021](0021-in-process-event-bus-and-choreography.md)
- Finding **A2** in [the architecture review backlog](../audits/2026-11-architecture-review.md).

---

*Part of the [Seiva documentation](../index.md).*
