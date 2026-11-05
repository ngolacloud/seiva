# Part 1: Your first module — a polls app

This tutorial builds the same application as the official
[Django tutorial](https://docs.djangoproject.com/en/6.1/intro/tutorial01/): a
small **polls** service. You can ask questions, give each question a few choices,
vote on a choice and read the results.

Where Django drives the app with models, views, templates and the admin, Seiva is
**API-first and compiled**: a module exposes **one machine-readable contract** and
the HTTP routes, the OpenAPI document and the gRPC service are all *derived* from
it. The finished code for this tutorial is the
[`examples/polls`](https://github.com/ngolacloud/seiva/tree/main/examples/polls)
module in the repository.

In this part you scaffold the project and the `polls` module, and meet the five
layers. (Later parts add validation, the database, authentication, tests and
packaging.)

## Create the project

The Django tutorial starts with `django-admin startproject mysite`. The Seiva
equivalent:

```console
$ seiva project new polls-site
$ cd polls-site
```

This is a cargo workspace with a composition root (`apps/polls-site-api`), a shared
kernel, a `schema/` and the cloud-native packaging. It compiles standalone.

## Add the polls module

Django's `python manage.py startapp polls` creates an app; Seiva's equivalent
creates a **module**:

```console
$ seiva module new polls
```

This generates `modules/polls/` as **five crates**, each a layer, and auto-wires
the module into the composition root (its router is merged and its deps added):

- **`facade`** — the versioned contract: a `#[seiva::facade] trait Polls` plus the
  wire DTOs (`CreateQuestion`, `QuestionDto`, `ChoiceDto`, …) with `serde`,
  `Validate` and `WireSchema`. This is the Seiva home of what Django puts in
  `models.py` *and* the view signatures.
- **`domain`** — the entities (`Question`, `Choice`) and their rules (a question's
  and a choice's text must not be empty). Depends on nothing but the shared kernel.
- **`application`** — the use cases implementing the facade over *ports*
  (`PollRepository`, `IdGenerator`, `UnitOfWork`, `EventPublisher`). These are the
  Seiva equivalent of Django's views: `list` is the index view, `detail`/`results`
  are the detail/results views, `vote` bumps a choice's tally. Transport- and
  database-free.
- **`infrastructure`** — adapters: an in-memory `PollRepository` by default (a
  DORM-backed one arrives in Part 3).
- **`interface`** — the `#[seiva::controller]` that turns the facade into HTTP
  routes, plus the module's OpenAPI.

The compiler enforces the dependency direction, so the architecture cannot rot
(see [Architecture](../../architecture.md)).

## The contract you are building

By the end of the tutorial the `polls` facade reads like this (the full file is
[`examples/polls/facade`](https://github.com/ngolacloud/seiva/tree/main/examples/polls/facade)):

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

```console
$ seiva start            # add --watch for hot-reload (needs cargo-watch)
```

The service serves `/health`, `/openapi.json`, `/docs`, plus the module's routes.
Confirm the contract documents your module:

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
