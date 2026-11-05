# Part 6: Packaging and deployment

Your project was born cloud-native. In this part we build its image and see how it
runs in production.

## The image

`seiva project new` generated a `Delonixfile` — a multi-stage, **distroless**,
non-root build:

```dockerfile
FROM rust:1-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin notes-app-api

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /build/target/release/notes-app-api /app/notes-app-api
ENV SEIVA_HTTP_ADDR=0.0.0.0:8080
ENV APP_ENV=production
USER nonroot
ENTRYPOINT ["/app/notes-app-api"]
```

The final image carries **only the binary and its runtime libraries** — no shell,
no package manager — and runs as uid 65532. Deploy is "copy one binary", not "ship
a Python/Node runtime + dependencies".

## Build and run

```console
$ delonix stack up        # or `docker compose up` — builds the Delonixfile
```

The generated `compose.yaml` builds from the `Delonixfile` and maps `8080`.
Optional backing services (a database, a broker) live in `compose/*.yaml` and are
opted in explicitly.

## Health and readiness

The service exposes `/health/live` and `/health/ready` over a
`Starting → Ready → Draining` lifecycle. Distroless has no shell for a CMD
healthcheck, so Kubernetes uses an httpGet readiness probe on `/health/ready`. On
`SIGTERM` the service drains in-flight requests before exiting — a rollout is
graceful.

## Configuration

Everything is environment (12-factor): `SEIVA_HTTP_ADDR`, `APP_ENV`, the OIDC
variables, and `DELONIX_ORM_DSN` for the data layer. The platform injects secrets;
nothing is baked into the image. See [Configuration](../../topics/configuration.md).

## You have a service

From `seiva project new` to a distroless, authenticated, database-backed,
self-documenting service — this is the whole loop. Next: the
[topic guides](../../topics/index.md) for depth, or the
[competitive landscape](../../competitive-landscape.md) for where Seiva stands.

---

*Part of the [Seiva documentation](../../index.md).*
