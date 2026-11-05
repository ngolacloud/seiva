# Part 1: Your first module

We will build a `notes` service. In this part you scaffold the project and a
module, and meet the five layers. (Later parts add validation, the data layer,
authentication, tests and packaging.)

## Create the project

```console
$ seiva project new notes-app
$ cd notes-app
```

This is a cargo workspace with a composition root (`apps/notes-app-api`), a shared
kernel, a `schema/` and the cloud-native packaging. It compiles standalone.

## Add a module

```console
$ seiva module new notes
```

This generates `modules/notes/` as **five crates**, each a layer, and auto-wires
the module into the composition root (its router is merged and its deps added):

- **`facade`** — the versioned contract: a `#[seiva::facade] trait Notes` plus the
  wire DTOs (`CreateRecord`, `RecordDto`) with `serde`, `Validate` and `WireSchema`.
- **`domain`** — the entity (`Record`) and its rule (a title must not be empty).
  Depends on nothing but the shared kernel.
- **`application`** — the use case implementing the facade over *ports*
  (`RecordRepository`, `IdGenerator`). Transport- and database-free.
- **`infrastructure`** — adapters: an in-memory `RecordRepository` by default (a
  DORM-backed one arrives in Part 3).
- **`interface`** — the `#[seiva::controller]` that turns the facade into HTTP
  routes, plus the module's OpenAPI.

The compiler enforces the dependency direction, so the architecture cannot rot
(see [Architecture](../../architecture.md)).

## Run it

```console
$ seiva start            # add --watch for hot-reload (needs cargo-watch)
```

The service serves `/health`, `/openapi.json`, `/docs` and `/admin`, plus the
module's route `POST /v1/notes`. Confirm the contract documents your module:

```console
$ seiva openapi | grep -o '"/v1/notes"'
"/v1/notes"
```

That route, its request/response schema and its `notes.create` permission were all
derived from the facade — you wrote no routing code.

## Next

[Part 2: Validation and DTOs](part-2-validation-and-dtos.md) — make the write
body validated and honest.

---

*Part of the [Seiva documentation](../../index.md).*
