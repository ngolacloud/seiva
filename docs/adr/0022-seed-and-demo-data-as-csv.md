# ADR 0022 — Per-module seed and demo data as CSV

- Status: Accepted
- Date: 2026-11-09
- Deciders: NgolaCloud
- Follows: ADR 0001, ADR 0002, ADR 0003, ADR 0021

## Context

A project needs two kinds of pre-loaded data: **reference/seed data** that every
environment must have (lookup tables, default records), and **demo data** a
developer boots a service with to see it working. Odoo solves this with per-addon
`data/` (always loaded) and `demo/` (dev-only) CSV files, loaded through the ORM
so constraints fire and rows carry a stable *external id* for idempotency and
cross-references.

Seiva's constraints shape the adaptation:

- A **module is five layers and owns its data** (ADR 0003). Seed data for a module
  belongs *with the module*, so `seiva module split` carries it into the
  standalone service.
- **Use cases are one transaction each and modules cooperate through events** (ADR
  0021). Loading rows through a facade would publish domain events and fire the
  choreography — desirable for a *demo*, wrong for bulk *reference* data.
- The default build is **DORM-free and runs on in-memory adapters** with no
  database and no new dependencies (the `cargo deny` supply-chain gate).
- A distroless image has **no source tree at runtime**, so CSV files must be
  carried in the binary, not read from disk.
- Ids are a 128-bit `Id([u8;16])` minted at create time; there is **no external-id
  concept** to make a load idempotent.

## Decision

1. **Per-module CSV folders.** A module ships `modules/<name>/seed/*.csv`
   (reference data) and `modules/<name>/demo/*.csv` (demonstration data). They live
   under `modules/<name>/`, so `seiva module split` carries them to
   `apps/<name>-service` unchanged.

2. **Hybrid loading.**
   - **Seed** loads on **every boot**, through the **data layer** with an
     **idempotent upsert**, and **publishes no events**. It is reference data, not
     a business transaction.
   - **Demo** loads **only in dev** (`AppEnv::is_development()`, or an explicit
     `SEIVA_LOAD_DEMO`), **through the module facade**, so each row runs the real
     use case — the domain invariants, and the domain event where the facade
     publishes one (so a demo can exercise the choreography after a `drain`).

3. **Deterministic external id, no mapping table.** The CSV's first column `id` is
   a stable textual key. The entity id is `seed::external_id("<module>.<resource>",
   key)` — FNV-1a over the scope, a separator and the key, 128-bit, pure integer
   arithmetic. The same key always yields the same id (so a reseed upserts rather
   than duplicates) and one module's row can be referenced by another by key. This
   is Odoo's external id without an `ir.model.data` mapping table.

4. **Compile-time registry, applied at the composition root.** Each CSV is embedded
   with `include_str!` and declared with `register_seed!`
   (`crates/delonix-seiva/src/seed.rs`), mirroring the admin registry (ADR 0008):
   `registered_seeds()` makes the sources discoverable to a fitness test and
   `seiva db seed`. The *application* (upsert / facade call) is wired in the
   composition root `apps/<name>-api`, the only place that holds the concrete
   adapters — exactly like the module router merges. A `seiva module new` module
   generates: a repository keyed by id (so `save` is an idempotent upsert),
   `infrastructure::load_seed`, `interface::load_demo`, both CSV fixtures, and the
   boot step in `main`.

5. **A dependency-free CSV parser.** `seed::parse_csv` is an RFC 4180 subset
   (header row, quoted fields with `""` escaping, `#` comments), so the default
   build takes on no new supply-chain surface.

6. **`seiva db seed [--demo]`.** Runs the composition root with `SEIVA_SEED=1`
   (and `SEIVA_LOAD_DEMO=1` for `--demo`): the modules apply their seed, then the
   binary exits before serving — reusing the one boot wiring (as `SEIVA_EMIT` does
   for `seiva openapi`).

## Consequences

- Seeding runs on every boot and is idempotent, so a restart never duplicates
  reference data. Persistence is the adapter's: the default in-memory adapter
  makes seed ephemeral (a smoke test); a DORM-backed adapter persists it.
- A freshly generated project stays warning-clean: the dev gate (`let load_demo`)
  is inserted with the first module, not in the empty skeleton.
- The generated repository is now keyed by id — a strictly better in-memory
  adapter (the previous `Vec` appended, which would have duplicated on reseed).
- Fitness functions prove the shape: `generated_module_has_seed_and_demo`,
  `composition_root_wires_seed_on_boot`, and the existing
  `generated_project_compiles_standalone` (which now wires the module into `main`,
  so the boot-time seed path is actually compiled).
- Demo data through the facade only fires events where the facade already
  publishes one; the scaffold's generated use case does not yet open a transaction
  or publish (finding F2/P0), so richer demo choreography lands when that closes.

## Alternatives considered

- **Project-level `data/{seed,demo}/`.** Rejected: it breaks per-module data
  ownership and the `split` would have to move files.
- **Seed through the facade too (uniform path).** Rejected: bulk reference data
  firing domain events would trigger the choreography (e.g. seeding orders would
  reserve stock and capture payments). Seed is not a business transaction.
- **Load through raw SQL / direct DORM insert bypassing the domain.** Rejected for
  *demo* (it must exercise invariants); accepted in spirit for *seed*, which upserts
  a constructed domain entity through the repository port (still via `Record::new`,
  so the invariant holds), never raw SQL.
- **The `csv` crate.** Rejected for now to keep the default build dependency-free
  and the `cargo deny` gate green; the parser is a small RFC 4180 subset. Documented
  escape hatch: adopt `csv` if richer quoting is ever needed.
- **An `ir.model.data` mapping table (Odoo's approach).** Rejected as unnecessary:
  a deterministic hash of the external key gives idempotency and cross-references
  with no table to store, migrate or keep consistent.
