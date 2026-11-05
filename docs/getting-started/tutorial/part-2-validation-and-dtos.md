# Part 2: Validation and DTOs

The generated `notes` facade already has a write DTO. In this part we see how the
request body is validated and how that shows up — honestly — in the contract.

## The DTO

In `modules/notes/facade/src/lib.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, WireSchema)]
pub struct CreateRecord { pub title: String }

impl Validate for CreateRecord {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::new("title must not be empty"));
        }
        Ok(())
    }
}
```

- `serde` makes it the JSON body; `WireSchema` publishes its real fields to the
  OpenAPI; `Validate` is the input rule.

## How it behaves

The generated controller reads the body with `seiva_http::Valid<CreateRecord>`:
the body **is** the DTO (no `{cmd: …}` wrapper). So:

```console
# valid → 201 Created with the RecordDto
$ curl -s -XPOST localhost:8080/v1/notes -H 'content-type: application/json' \
    -d '{"title":"hello"}'

# invalid → 422 Problem (RFC 9457) with your message
$ curl -s -XPOST localhost:8080/v1/notes -H 'content-type: application/json' \
    -d '{"title":"  "}'
{"type":"about:blank","title":"Unprocessable Entity","status":422,
 "detail":"title must not be empty"}
```

## It matches the docs

Because `CreateRecord` derives `WireSchema`, `/openapi.json` shows the body as
`{type:object, properties:{title:{type:string}}, required:["title"]}` — exactly
what `Valid<CreateRecord>` accepts. No drift between the docs and the server (ADR
0007). Add a field to the DTO, and the contract follows.

## Add a field (optional exercise)

Add `pub body: String` to `CreateRecord` (and to the domain `Record`), restart,
and check `/openapi.json` — the new required field appears, and `Valid` enforces
it.

## Next

[Part 3: Persisting with the data layer](part-3-the-data-layer.md) — move from the
in-memory adapter to a real database.

---

*Part of the [Seiva documentation](../../index.md).*
