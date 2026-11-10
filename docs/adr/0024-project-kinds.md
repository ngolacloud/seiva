# ADR 0024 — Project kinds: an advisory `seiva project new --kind`

- Status: Accepted
- Date: 2026-11-09
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0003, ADR 0012, ADR 0021

## Context

`seiva project new <name>` generated a single skeleton — an HTTP+gRPC API server
hosting modules — regardless of what the developer was building. The architecture
review (finding F3) flagged this: the bootstrap does not adapt to the project's
nature and does not advise. Not every service is a request/response API: some are
background workers, some are event-processors reacting to the event bus (ADR 0021),
some are HTTP edges/BFFs aggregating upstreams. Generating the wrong shape teaches
the wrong starting point — and the generated skeleton is the most-read documentation.

## Decision

1. **Four project kinds, chosen with `--kind`:** `seiva project new <name>
   [--kind api|worker|gateway|event]`. Each generates the cloud-native composition
   root that fits that nature; `api` is the default.
   - **api** — an HTTP+gRPC server that hosts the project's modules (the existing
     shape): `serve_router` + `router_with_modules` + `/health` + `/openapi.json`
     + `/docs`, the `SEIVA_EMIT` contract path, the seed/demo step (ADR 0022), and
     the managed module/extension composition anchors.
   - **worker** — a background service over the framework's worker host
     (`delonix_seiva::worker::{run_worker, WorkerConfig}` + `shutdown_signal`), no
     HTTP surface. A `pass` stub the developer replaces with the job.
   - **gateway** — an HTTP edge/BFF shell: the framework surface (health, security
     headers, authn) with **no modules of its own** and a marked place to add
     upstream-aggregation routes (the developer brings an HTTP client).
   - **event** — an event-processor: a worker whose pass drains the in-process
     event bus (`seiva_events::drain_in_process`) and dispatches to registered
     subscribers, the same choreography a monolith runs (ADR 0021). In-memory
     outbox/inbox by default; the DORM transactional outbox + a durable inbox in
     production.

2. **Advisory, not interactive.** Without `--kind` the CLI defaults to `api` **and
   prints guidance** listing the four kinds and what each is for. No interactive
   prompt — the CLI stays non-interactive and reproducible in CI.

3. **The composition-root crate is `apps/<name>-api` for every kind.** `-api` reads
   as "the application entrypoint", not "HTTP". The `kind` is recorded in
   `seiva.toml` (`[project] kind = "..."`); only `main.rs`, the Delonixfile/compose
   (ports vs headless), the README and the advice vary. This keeps the whole
   `start`/`openapi`/`db seed`/`module new`/`module split`/`ext add` toolchain —
   which resolves `<name>-api` — working unchanged.

4. **Only `api` hosts modules.** It alone emits `schema/`, `modules/` and the
   module/extension composition anchors. `seiva module new` in a worker/gateway/event
   project fails with the existing `MissingAnchor` error (those kinds do not host
   business modules).

5. **Every kind stays cloud-native** (ADR 0012): distroless, non-root, 12-factor
   config, graceful shutdown. Worker/event images carry no HTTP port.

## Consequences

- The bootstrap now guides a developer to the right shape for the project's nature
  without giving up cloud-native defaults — closing F3.
- Each kind reuses a framework piece already shipped (serve loop, worker host,
  event bus), so no new runtime and no new external dependency — `cargo deny` stays
  green. The `event` kind's in-memory inbox is `seiva-testing`'s double, marked
  dev-only (replace with the DORM inbox in production).
- A fitness function (`generated_project_compiles_for_each_kind`) compiles a
  `worker`/`gateway`/`event` project against the checkout, so a drifted worker-host
  or event-bus signature is caught like the `api` shape is by
  `generated_project_compiles_standalone`.
- The interim `[patch.crates-io]` lists every framework crate; a non-api project
  does not use them all, so `cargo build` prints benign "patch not used" warnings
  until the crates are published and the patch is removed.

## Alternatives considered

- **Name the crate `apps/<name>-<kind>`.** Rejected: it would force the
  module/split/ext/start/openapi tooling to resolve the kind-specific name,
  destabilising a working web for marginal naming benefit. The `kind` in
  `seiva.toml` records the nature without moving the folder.
- **Interactive prompt when `--kind` is omitted.** Rejected: it breaks CI/automation
  and needs a TTY, against the CLI's non-interactive style. A default + printed
  advice gives the same guidance.
- **A real upstream-aggregation client in `gateway`.** Deferred: it would pull an
  HTTP client (reqwest/hyper) and a routing pattern into the scaffold; the gateway
  ships a documented shell and the developer chooses the client.
