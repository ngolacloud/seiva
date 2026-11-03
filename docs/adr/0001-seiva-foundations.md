# ADR 0001 — Delonix Seiva foundations

- Status: Accepted
- Date: 2026-11-03
- Deciders: NgolaCloud

## Context

Delonix Seiva is a Rust backend framework for robust, high-criticality systems:
backend only, with the ease of FastAPI and NestJS, a production- and
cluster-ready server, and DORM (the Delonix ORM) as the default data layer. It
is a new product in its own repository, distinct from DORM, which stays an
independent ORM (DORM ADR 0073).

Three foundational decisions must be fixed before any module or runtime is
built, because every later choice depends on them: where the code lives, where
the service contract originates, and which message bus is the default.

## Decision

### 1. Repository

Seiva lives in its own repository, `ngolacloud/seiva`, crate `delonix-seiva`,
CLI `seiva`. It is not a workspace inside DORM.

Seiva depends on DORM; DORM never on Seiva. While DORM has not yet published a
version with the stable library API, Seiva depends on DORM **by git with a
pinned revision** (never by local path), through the `seiva-data-dorm` gateway
only. Once DORM publishes the library API, Seiva depends on the published
crates, and the two products are kept compatible by a documented compatibility
matrix.

The framework core does not depend on DORM. The ORM enters only through the
`seiva-data-dorm` gateway behind the `dorm` feature (on by default). A project
whose every module uses an external data layer links no ORM at all.

### 2. Contract origin — Rust first

A module's public contract originates in Rust. The `#[seiva::facade]` attribute
on a trait is the single source of truth; from it Seiva generates the versioned,
reviewable `.proto`, the gRPC server and client, the OpenAPI document and the
contract tests. The generated `.proto` is committed and reviewable so a contract
change is visible in review.

A module may also start from an existing `.proto` when integrating with a system
that already owns the contract; in that case the generated Rust facade carries
the "generated, do not edit" header and the `.proto` is the source.

### 3. Default message bus — NATS JetStream

The default broker is NATS JetStream: it is operationally light, has first-class
Rust support, and gives durable streams with consumer acknowledgement, which is
what the outbox relay and the inbox need. Kafka is available behind a feature for
deployments that already run it or need its throughput and retention. AMQP
(RabbitMQ) stays available behind a feature as well.

The broker is per module: a module declares its bus, and the relay from that
module's outbox targets it. The messaging clients come from `seiva-messaging`
(the code moving out of DORM's `dorm-messaging`, with history preserved).

## Consequences

- The founding workspace carries three crates: `seiva-ports` (trait-only
  boundary, SemVer-stable by policy), `delonix-seiva` (the framework entrypoint)
  and `seiva-cli` (the `seiva` binary). None of them depends on DORM; the
  `dorm` feature is declared on `delonix-seiva` but pulls in nothing until the
  `seiva-data-dorm` gateway exists.
- The boundary between DORM and Seiva is governed by the "ORM boundary" rule
  (fixed in a DORM ADR under Parte DORM-C): a capability enters DORM only if it
  makes sense for someone using DORM with no framework at all. Everything whose
  guarantee comes from communication between services, from time, from caller
  identity or from policy is Seiva's and lives behind a port.
- Rust-first contracts mean the gRPC/OpenAPI/`.proto` artifacts are generated
  and never hand-edited (Generation Gap); the lint fails if they are.

## Alternatives considered

- **Seiva as a workspace inside DORM.** Rejected: it would couple the two
  release cadences and blur the one-way dependency. DORM must stay usable with
  no framework.
- **Protobuf-first contracts.** Rejected as the default: it pushes DTOs and
  validation out of Rust's type system, losing the "types are the source of
  everything" property. Allowed as an opt-in for `.proto`-owned integrations.
- **Kafka as the default bus.** Rejected as the default: heavier to operate for
  the common case. Kept behind a feature.
