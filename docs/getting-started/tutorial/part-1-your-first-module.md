# Part 1: Your first module — a polls app

This tutorial builds a small **polls** service — the canonical first app. You can
ask questions, give each question a few choices, vote on a choice and read the
results.

!!! info "Where this tutorial is going"
    You will build polls as a **modular monolith** first — one binary, one
    deploy — across Parts 1–6. Then, in
    [Part 7](part-7-monolith-to-microservice.md), you will promote the `polls`
    module to its **own microservice** with a single command, *without changing
    the module's code*. That arc — start simple, split when you need to — is the
    point of Seiva's module boundary.

Seiva is **API-first and compiled**: your handlers are plain `async fn`, a module exposes
**one machine-readable contract**, and the HTTP routes, the interactive API docs
and the gRPC service are all *derived* from it. The finished code for this
tutorial is the
[`examples/polls`](https://github.com/ngolacloud/seiva/tree/main/examples/polls)
module in the repository.

In this part you scaffold the project and the `polls` module, and meet the five
layers. (Later parts add validation, the database, authentication, tests and
packaging.)

## Create the project

Start by scaffolding the project. The `seiva` CLI creates a Cargo workspace — a
modular monolith — with a composition root, a shared kernel, a `schema/` and the
cloud-native packaging:

```console
$ seiva project new polls-site
$ cd polls-site
```

It compiles standalone. From here on you can drive the build with either Cargo or
[**`dx`**](https://ngolacloud.github.io/dx/) (Delonix's Cargo-compatible package
manager); the tabs below show both — pick one.

=== "dx"

    ```console
    $ dx build          # compile the workspace
    ```

=== "cargo"

    ```console
    $ cargo build       # compile the workspace
    ```

## Add the polls module

Now add the `polls` **module**:

```console
$ seiva module new polls
```

This generates `modules/polls/` as **five crates**, each a layer, and auto-wires
the module into the composition root (its router is merged and its deps added):

- **`facade`** — the versioned contract: a `#[seiva::facade] trait Polls` of
  `async fn`s, plus the wire DTOs (`CreateQuestion`, `QuestionDto`, `ChoiceDto`, …)
  with `serde`, `Validate` and `WireSchema`. This is where the module's data
  shapes *and* its method signatures live.
- **`domain`** — the entities (`Question`, `Choice`) and their rules (a question's
  and a choice's text must not be empty). Depends on nothing but the shared kernel.
- **`application`** — the use cases implementing the facade over *ports*
  (`PollRepository`, `IdGenerator`, `UnitOfWork`, `EventPublisher`). These are the
  request handlers: `list` returns the latest questions, `detail`/`results` return
  a question with its choices, `vote` bumps a choice's tally. Transport- and
  database-free.
- **`infrastructure`** — adapters: an in-memory `PollRepository` by default (a
  DORM-backed one arrives in Part 3).
- **`interface`** — the `#[seiva::controller]` that turns the facade into HTTP
  routes, plus the module's OpenAPI.

The compiler enforces the dependency direction, so the architecture cannot rot
(see [Architecture](../../architecture.md)).

## The project layout

After scaffolding the project and adding the module, your workspace looks like
this:

```text
polls-site/
├── Cargo.toml                   # the cargo workspace
├── seiva.toml                   # project configuration
├── apps/
│   └── polls-site-api/          # the composition root (the binary)
│       └── src/main.rs
├── shared/
│   └── kernel/                  # the shared kernel (ids, errors, time)
├── modules/
│   └── polls/                   # your module — five layers, one crate each
│       ├── facade/              #   the versioned contract + wire DTOs
│       ├── domain/              #   entities (Question, Choice) and rules
│       ├── application/         #   use cases over ports (transport/DB-free)
│       ├── infrastructure/      #   adapters (in-memory now, DORM in Part 3)
│       └── interface/           #   the controller + the module's OpenAPI
├── schema/
│   └── polls-site.schema        # the DORM schema (Part 3)
├── Delonixfile                  # the distroless image (Part 6)
└── compose.yaml                 # local run (Part 6)
```

Everything under `modules/` is yours; `apps/polls-site-api` is the thin root that
composes the modules into one binary. Adding another module with
`seiva module new <name>` creates another `modules/<name>/` and wires it in — the
project grows by *adding* folders, never by editing a central registry.

## The contract you are building

By the end of the tutorial the `polls` facade reads like this (the full file is
[`examples/polls/facade`](https://github.com/ngolacloud/seiva/tree/main/examples/polls/facade)).
Note every method is `async fn` — handlers are asynchronous by default:

```rust
#[facade(version = "v1")]
pub trait Polls {
    #[requires("polls.manage")]
    async fn ask(&self, cmd: CreateQuestion) -> Result<QuestionDto, String>;

    #[requires("polls.manage")]
    async fn add_choice(&self, cmd: AddChoice) -> Result<ChoiceDto, String>;

    #[requires("polls.read")]
    async fn list(&self) -> Result<Vec<QuestionDto>, String>;          // the index view

    #[requires("polls.read")]
    async fn detail(&self, q: GetQuestion) -> Result<QuestionDetail, String>;

    #[requires("polls.vote")]
    async fn vote(&self, cmd: Vote) -> Result<ChoiceDto, String>;      // the vote() view

    #[requires("polls.read")]
    async fn results(&self, q: GetQuestion) -> Result<QuestionDetail, String>;
}
```

Each method names the permission a caller must hold — the transport enforces it
(Part 4). You wrote no routing code; the routes come from this trait.

## Run it

=== "dx"

    ```console
    $ seiva start                 # dev loop (add --watch for hot-reload)
    # or directly: dx run -p polls-site-api
    ```

=== "cargo"

    ```console
    $ seiva start                 # dev loop (add --watch for hot-reload)
    # or directly: cargo run -p polls-site-api
    ```

The service serves `/health`, `/openapi.json`, the interactive docs
(`/docs` = Swagger UI, `/redoc` = ReDoc, `/scalar` = Scalar), plus the module's
routes. Open <http://localhost:8080/docs> and the whole `polls` contract is
already documented — you wrote no spec. Confirm it from the shell too:

```console
$ seiva openapi | grep -o '"/v1/polls"'
"/v1/polls"
```

That route, its request/response schema and its `polls.*` permissions were all
derived from the facade — no view functions, no URLconf, no serializers written by
hand.

## Next

[Part 2: Validation and DTOs](part-2-validation-and-dtos.md) — make the question
and choice bodies validated and honest.

---

*Part of the [Seiva documentation](../../index.md).*
