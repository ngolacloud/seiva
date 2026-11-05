# Requests and responses

How a request flows through a Seiva service, and how results and errors come back
on the wire.

## The lifecycle

A request enters the framework layers (outermost first), reaches a module route,
and the generated handler runs the use case:

```
request → request-id → security headers → authn → rate limit → CORS →
          body-size limit → route handler → facade method → response
```

The layers are covered in [middleware and the layer stack](middleware-layers.md);
this page is about what the handler reads and returns.

## Reading the request

The `#[seiva::controller]` handler extracts exactly what the method needs:

- a **write** body is deserialised and validated as the facade's DTO
  (`seiva_http::Valid<T>`) — the body **is** the DTO (see [Validation](../validation.md));
- a **`get`** reads the id from the path; a **`list`** reads an optional query.

An oversized body (over the 1 MiB default) is rejected before the handler runs.

## Responses

- **Success** — the facade's `Ok` value is serialised to JSON with the
  convention's status: `201 Created` for `create`, `200 OK` otherwise.
- **Failure** — the facade's error maps to an RFC 9457
  [`Problem`](../../reference/crates/index.md) (`application/problem+json`): a
  machine-readable error with a `type`, `title`, `status` and `detail`. Validation
  failures become `422 Unprocessable Entity`; an unmet `#[requires]` is `403`
  (or `401` when a bearer was supplied but invalid).

Every response — including a short-circuited one (a `429`, a `401`, a CORS
preflight) — carries the secure headers and a correlation `request-id`, because
those layers are outermost.

## Content type

Requests and responses are JSON. The DTOs derive `serde`, and
[`WireSchema`](../../reference/wire-schema.md) publishes their real shapes into the
OpenAPI document, so a client codes against the exact bodies the server accepts
and returns.

## Related

- [Controllers and routing](controllers-and-routing.md) · [Middleware and layers](middleware-layers.md)
- [Validation](../validation.md) · [Security](../security.md)

---

*Part of the [Seiva documentation](../../index.md).*
