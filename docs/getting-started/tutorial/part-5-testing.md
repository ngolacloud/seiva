# Part 5: Testing your module

A polls app deserves tests. The `polls` module ships with them, and this part
shows the three levels Seiva makes easy: use-case unit tests, the architecture
fitness functions, and how to run them.

## Unit-test the use case (no server, no DB)

The application layer codes against ports, so the use case tests with in-memory
fakes from `seiva-testing` — no HTTP, no database. This is the real test from
[`examples/polls/application`](https://github.com/ngolacloud/seiva/tree/main/examples/polls/application),
condensed:

```rust
#[tokio::test]
async fn ask_add_choice_vote_and_read_back() {
    let svc = service();                                   // fakes for every port

    let q = svc.ask(CreateQuestion { text: "What's new?".into() }).await.unwrap();
    let a = svc.add_choice(AddChoice { question_id: q.id, text: "Not much".into() }).await.unwrap();

    svc.vote(Vote { choice_id: a.id }).await.unwrap();
    let after = svc.vote(Vote { choice_id: a.id }).await.unwrap();
    assert_eq!(after.votes, 2);                            // the tally is real

    let detail = svc.detail(GetQuestion { id: q.id }).await.unwrap();
    assert_eq!(detail.choices[0].votes, 2);
}
```

The suite also proves the unhappy paths: an empty question is refused by the
domain rule (nothing commits), a choice for a missing question errors, and voting
an unknown choice **rolls the transaction back** and emits no event. Run them:

```console
$ cargo test
```

## Architecture fitness functions

`seiva lint` runs the invariants that keep the architecture honest — for example,
the `interface` layer may not depend on `infrastructure`, and a module's
`application` must not name `delonix_seiva` directly:

```console
$ seiva lint
```

These fail the build when a layering rule is broken, so the five-layer design
cannot erode (see the `seiva-pattern-fitness` catalogue).

## Controller / HTTP tests

The framework's own `seiva-controller-test` proves a `#[seiva::controller]`
generates a working, guarded, versioned router end to end — the pattern to copy
when you want to test the polls routes with a test client (a `403` with no token,
`201` for a valid `ask`, the vote tally rising).

## The principle

Put input validation in `Validate`, business rules in the `domain` (the vote
tally, the non-empty text — unit-tested with no dependencies), and keep the
application layer transport-free so its tests need nothing but fakes. The
expensive things (a database, a server) are for the few integration tests, not the
many unit tests. See [Testing](../../topics/testing.md).

## Next

[Part 6: Packaging and deployment](part-6-packaging-and-deploy.md).

---

*Part of the [Seiva documentation](../../index.md).*
