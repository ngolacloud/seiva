# Controllers and routing

A **controller** turns a facade implementation into HTTP routes. You annotate the
`impl` with `#[seiva::controller]`; the macro generates a `<trait>_router(service)`
builder — you never write a route or a URLconf.

```rust
#[seiva::controller]
impl<R, G> Notes for NotesController<R, G>
where R: RecordRepository + Send + Sync + 'static,
      G: IdGenerator + Send + Sync + 'static,
{
    async fn create(&self, cmd: CreateNote) -> Result<NoteDto, String> { /* … */ }
}
// generates: pub fn notes_router(service) -> axum::Router
```

## The routing convention

The method name chooses the verb and path; the resource is the facade name,
lowercased and pluralised; the version is the descriptor's:

| method | HTTP | path | request |
| --- | --- | --- | --- |
| `create` | `POST` | `/{version}/{resource}` | the DTO body (`201`) |
| `get` | `GET` | `/{version}/{resource}/{id}` | id from the path |
| `list` | `GET` | `/{version}/{resource}` | optional query |
| *other* | `POST` | `/{version}/{resource}/{method}` | the DTO body |

## The generated handler

For each route the handler pulls the injected service from axum state, extracts
the request, calls the facade method, and maps the result:

- **write** methods read the single body parameter with `seiva_http::Valid<T>` —
  a validated JSON body (see [Validation](../validation.md)); the body **is** the
  DTO, so the OpenAPI documents it directly (no `{cmd: …}` envelope).
- **`get`** reads the id with `axum::extract::Path<T>`; **`list`** an optional
  `axum::extract::Query<T>`.
- the `Ok` value is serialised to JSON; the `Err` maps to an RFC 9457 `Problem`.

## Per-route permission

Each route's required permission is the method's `#[requires("…")]`, read from the
descriptor at run time and enforced by `seiva_http::permission_guard` — never
hard-coded. An empty `requires` is a public route.

## Composition

The controller is generic over its ports, so the `interface` layer never names a
concrete adapter — the composition root in `apps/<name>-api` injects them and
merges the router (`seiva module new` auto-wires this). The merge happens *before*
the framework layers, so authn/security/rate-limit apply to module routes — see
[the application and composition root](application-and-composition.md) and
[middleware and layers](middleware-layers.md).

## Related

- [Facades](facades.md) · [Validation](../validation.md) · [API documents](../api-documents.md)

---

*Part of the [Seiva documentation](../../index.md).*
