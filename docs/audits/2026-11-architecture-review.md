# Architecture review — findings backlog (2026-11)

Panel: **Martin Fowler** + **Robert C. Martin (Uncle Bob)**, guest **Eric Evans**
(skill `seiva-architecture-reviewers`). The review found the architectural spine
sound — five layers with the dependency rule enforced by the crate graph **and**
`seiva lint`, the contract as single source of truth, monolith↔microservice by
composition — and left the findings below. A finding is **closed** only when its
**closing fitness function** is green, not when the code merely looks right.

How to use this file: the `architecture-reviewer` agent reconciles every new
finding against this list; the `seiva-architecture-reviewers` and
`seiva-pattern-fitness` skills reference it. Update the **Status** column as each
item lands.

## Findings

| ID | Sev | Finding | Anchor | Status | Closing fitness function |
|----|-----|---------|--------|--------|--------------------------|
| **F1** | P0 | The contract layer depends on the HTTP transport: `facade` depends on `seiva-http` (`Problem` = RFC 9457, `Validate`). The innermost contract knows a delivery mechanism; for a dual-transport framework the error semantics are HTTP-shaped. | `scaffold.rs` `facade_cargo_toml`/`facade_lib`; `docs/adr/0003` | **Closed (scaffold)** — standing lint rule (A1) + HTTP semantics (C) still open | ✅ `Validate`/`ValidationError` **and** the RFC 9457 `Problem` now live in the transport-neutral `seiva-contract` (no axum; `http::StatusCode`). `seiva-http` re-exports `Problem` and renders it via a new `ProblemResponse` axum `IntoResponse` wrapper; the `#[controller]` macro error arm wraps through it; the internal extractors/middleware switched to `ProblemResponse`. The facade (scaffold + 6 examples) now names **no** transport crate; `generated_facade_is_framework_wired` + `module_manifests_depend_on_the_right_framework_crates` assert it, and `cargo test -p seiva-http` proves the 422/413/403/429 responses still carry `problem+json`. The one transport type a facade may still name is `seiva_http::Ctx` (the request context) — a separate, narrower coupling, documented in ADR 0003. |
| **F2** | P0 | The generated use case has no transaction boundary and no domain event, contradicting the invariant "one use case = one transaction, modules cooperate through events" (ADR 0021). A developer copies the too-simple shape. | `scaffold.rs` `application_lib` | **Closed** (this session) | ✅ `generated_application_implements_the_facade_over_ports` now asserts the use case opens `UnitOfWork::transaction` and calls `events.publish("notes.record.created", …)` inside it; the generated unit test asserts `TxOutcome::Committed` + emitted topics via the `seiva-testing` doubles, and `generated_project_compiles_standalone` compiles it (`cargo build --tests`). |
| **F3** | P1 | The bootstrap is not adaptive to the project's nature: `project new` is one fixed skeleton, no profile/kind (API / worker / gateway / event-processor), no advisory prompt. | `seiva-cli` `project new` | **Closed** (this session) | ✅ `seiva project new <name> --kind api\|worker\|gateway\|event` (ADR 0024): each generates the fitting cloud-native composition root (worker host / event-bus drain / HTTP edge / module-hosting API). Without `--kind` it defaults to `api` and prints advice. Proven by `generated_project_compiles_for_each_kind` (worker/gateway/event compile standalone) + a live run of each. |
| **F4** | P2 | DDD vs Clean Arch choice: the `Repository` port lived in `application` (Clean Arch), not in `domain` (classic DDD/Evans). | `docs/adr/0003` §4; `scaffold.rs` `domain_lib`/`application_lib` | **Closed** (this session) | ✅ Moved to classic DDD: the repository port is now **defined in the `domain`**, framed in a domain `RepositoryError`; the `application` **re-exports** it (so `interface` still imports from `application`, the layer rule + domain purity unchanged — no `lint` change) and adapts the error to `PortResult`. The framework's technical ports (UoW/EventPublisher/…) stay in `application`. Scaffold + the canonical `examples/notes` migrated; `generated_application_implements_the_facade_over_ports` asserts the port lives in `domain` and `generated_project_compiles_standalone` compiles it. ADR 0003 §4 updated (reversed). **Follow-up:** the other 7 example modules (accounts, polls, shop/*) still use the prior layout — a tracked mechanical migration. |
| **F5** | P1 | `module split` does not address data ownership: the generated service mounts in-memory adapters, so the decentralized-data half of microservices is missing. | `scaffold.rs` `service_files`/`service_main_rs` | **Closed** (this session) | ✅ `module split` now scaffolds the service as data-owning: `apps/<name>-service/compose.yaml` brings up a database the service alone connects to (`<name>-db`) with a per-service `DELONIX_ORM_DSN`, plus a README spelling out the ownership; the module's `seed/` loads into THIS service's store (ADR 0022). Asserted by `split_scaffolds_a_single_module_service_binary` + proven live. (Turning on the `dorm` feature persists to the service's own DB; the in-memory adapter is the dev default.) |
| **F6** | P2 | Imprecise vocabulary and one empirical rule: "facade" names the inbound port / Published Language (not the GoF Facade); `resource_of` pluralises naively (`category`→`categorys`). | `scaffold.rs` `resource_of`; the `facade` naming | **Closed** (this session) | ✅ `resource_of` now delegates to the authoritative `crate::contract::resource_segment` (one rule; `Category`→`categories`), asserted by `generated_facade_route_uses_the_shared_pluralisation`. ✅ Vocabulary note added to the `#[facade]` macro doc and the generated facade header: it is the module's inbound port / Published Language, not the GoF Facade. |

## New findings — scaffold re-review (2026-11-15)

Panel re-review of the scaffold as it stands, adversarial on the F1–F6 "Closed"
claims. The spine holds (F2 opens the transaction + publishes inside it; F4 put
the repository port in `domain`). These are **new**, not in the table above. A
finding closes only when its **closing fitness function** is green.

| ID | Sev | Reviewer / lens | Finding | Anchor | Closing fitness function |
|----|-----|-----------------|---------|--------|--------------------------|
| **A1** | Alto | Uncle Bob — dependency rule | `seiva lint` has **no rule** for transport crates in the `facade`: `forbidden_layer_dep("facade", …)` only refuses DORM + sibling-layer crates, so a hand-written facade that depends on `seiva-http` passes green. F1's "transport-neutral" invariant is enforced **only** by a scaffold snapshot test (`module_manifests_depend_on_the_right_framework_crates`), not by the standing fitness. ADR 0003 further *permits* `facade → seiva_http::Ctx`, so the real position is "neutral by default, permitted in fact". | `lint.rs:178-183`; `scaffold.rs:2582` | A `seiva lint` rule refusing transport crates (`seiva-http` except the named `Ctx` exception, `axum`, `seiva-grpc`) in `facade`, with a fixture test that a facade depending on `axum` fails lint. |
| **A2** | Alto | Evans + Fowler — Published Language / domain model | The inter-module event is **schemaless**: the scaffold publishes `serde_json::json!({…})` under a string topic `"{snake}.record.created"`, consumed as a `CloudEvent` with `event.ty` a string. The channel the framework declares as the primary coupling is the one thing with no contract — while the synchronous facade DTOs carry `WireSchema` and reach OpenAPI. A consumer module has nothing to compile against. | `scaffold.rs:1617` (publish); `scaffold.rs:1251` (consume) | A typed, `WireSchema`-bearing event type with a versioned topic, link-time registered like facades; a fitness that the generated event payload matches a registered schema. Requires **ADR 0027** first. |
| **B** | Médio | Fowler — fitness functions / distribution | "Compiles standalone" rests on a local-path `[patch.crates-io]`: the framework version `0.1` is not on crates.io, so every generated project embeds an **absolute path to the generator's checkout**, and `generated_project_compiles_standalone` "resolves its own" root — green by construction, proving a portability the artefact lacks (it only builds where that path resolves). | `scaffold.rs:1445-1476` (`workspace_patch_section`); `FRAMEWORK_VERSION` = `"0.1"` | Publish the framework crates and delete the `[patch]` section (the comment says it is "deletable in a single edit"); fitness compiles the project **without** the patch. Until then, rename the test `..._against_local_checkout` and say so in the generated README. |
| **C** | Médio | Uncle Bob + Fowler — honest coupling | F1 moved `Problem`/`Validate` from `seiva-http` to `seiva-contract`, but `Problem` **is** RFC 9457 and carries `http::StatusCode`; the scaffolded facade maps its error with `Problem::unprocessable_entity` (422). For a dual-transport framework the innermost contract still speaks HTTP status codes — the coupling moved from a crate name to a semantic, less visible, not removed. *Contested:* ADR 0003 argues `StatusCode` is "the protocol-level status HTTP and gRPC share" (plausible). | `scaffold.rs:1543` (`Problem::unprocessable_entity`); ADR 0003 | Either a framework-domain error enum each transport maps to its own code (HTTP 422 / gRPC `INVALID_ARGUMENT`), or an ADR note that F1's neutrality is of crate name, not semantics. |
| **D** | Baixo | Fowler — evolutionary architecture | `project new --kind gateway\|event\|worker` offers distributed topologies on day zero, a mild MonolithFirst tension (the developer picks a topology before owning a monolith to split). Mitigated: default is `api` with printed advice, and they are composition roots, not forced microservices. | `scaffold.rs:239` (`main_rs_for`); ADR 0024 | Advisory text in the `gateway`/`event` headers: "use only when downstream services exist; otherwise start `api`". |

## Closed / delivered this session

- **ADR 0022 — per-module seed & demo data as CSV** (Odoo-inspired, hybrid load).
  Delivered end to end: `seed.rs` core (external-id, `parse_csv`, registry), the
  scaffold (idempotent keyed repo, embedded `seed/`+`demo/` CSV, `load_seed`/
  `load_demo`, boot step), `seiva db seed [--demo]`. Proven by the fitness tests
  (`generated_module_has_seed_and_demo`, `composition_root_wires_seed_on_boot`,
  `generated_project_compiles_standalone` now wiring the module into `main`) and a
  live boot (`APP_ENV=development SEIVA_SEED=1 SEIVA_LOAD_DEMO=1` → "seed/demo
  sources applied", exit 0).
- **Architecture review tooling**: the `seiva-architecture-reviewers` skill
  (Fowler + Uncle Bob + Evans) and the `architecture-reviewer` agent, so the
  findings close non-empirically.

## Deferred (tracked, not a finding)

- **Examples enrichment**: give the `examples/notes` module a `seed/`+`demo/`
  reference (idempotent-seed + demo-via-facade test) and prove demo-via-facade
  choreography on `examples/shop` (demo → `drain_in_process` → `orders.placed` →
  `inventory.reserved`). The feature is proven at the scaffold (what new modules
  get); this adds the hand-written reference developers copy from.
