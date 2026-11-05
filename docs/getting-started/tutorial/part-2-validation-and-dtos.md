# Part 2: Validation and DTOs

The Django tutorial's [Part 2](https://docs.djangoproject.com/en/6.1/intro/tutorial02/)
introduces the `Question` and `Choice` **models** and the admin. In Seiva the
request/response shapes are **DTOs** on the facade, and this part shows how a body
is validated and how that shows up — honestly — in the contract.

## The DTOs

In `modules/polls/facade/src/lib.rs` the write side of a question is a DTO:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateQuestion { pub text: String }

impl Validate for CreateQuestion {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.text.trim().is_empty() {
            return Err(ValidationError::new("question text must not be empty"));
        }
        Ok(())
    }
}
```

- `serde` makes it the JSON body; `WireSchema` publishes its real fields to the
  OpenAPI; `Validate` is the input rule. `AddChoice { question_id, text }` and
  `Vote { choice_id }` follow the same pattern.

## How it behaves

The generated controller reads the body with `seiva_http::Valid<CreateQuestion>`:
the body **is** the DTO (no `{cmd: …}` wrapper). So:

```console
# valid → 201 Created with the QuestionDto
$ curl -s -XPOST localhost:8080/v1/polls/questions -H 'content-type: application/json' \
    -d '{"text":"What is new?"}'
{"id":1,"text":"What is new?"}

# invalid → 422 Problem (RFC 9457) with your message
$ curl -s -XPOST localhost:8080/v1/polls/questions -H 'content-type: application/json' \
    -d '{"text":"  "}'
{"type":"about:blank","title":"Unprocessable Entity","status":422,
 "detail":"question text must not be empty"}
```

The same rule lives in the domain too: `Question::new` rejects empty text, so a
command that slips past the edge is still refused by the entity (Part 5 tests
both). Validation is defence in depth, not a single gate.

## It matches the docs

Because `CreateQuestion` derives `WireSchema`, `/openapi.json` shows the body as
`{type:object, properties:{text:{type:string}}, required:["text"]}` — exactly what
`Valid<CreateQuestion>` accepts. No drift between the docs and the server (ADR
0007). Add a field to the DTO, and the contract follows.

## Add a field (optional exercise)

Django's `Question` has a `pub_date`. Add `pub published_at: String` to
`QuestionDto` (and carry it on the domain `Question`), restart, and check
`/openapi.json` — the new field appears in the response schema with no serializer
to update by hand.

## Next

[Part 3: Persisting with the data layer](part-3-the-data-layer.md) — move the
questions and choices from the in-memory adapter to a real database.

---

*Part of the [Seiva documentation](../../index.md).*
