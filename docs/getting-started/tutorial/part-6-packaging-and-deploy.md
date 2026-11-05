# Part 6: Packaging and deployment

Your polls project was born cloud-native. Where the Django tutorial ends with
`runserver` and leaves production as an exercise, Seiva compiles the whole service
to a single binary — so this part builds its image and shows how it runs in
production.

## The image

`seiva project new` generated a `Delonixfile` — a multi-stage, **distroless**,
non-root build:

```dockerfile
FROM rust:1-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin polls-site-api

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /build/target/release/polls-site-api /app/polls-site-api
ENV SEIVA_HTTP_ADDR=0.0.0.0:8080
ENV APP_ENV=production
USER nonroot
ENTRYPOINT ["/app/polls-site-api"]
```

The final image carries **only the binary and its runtime libraries** — no shell,
no package manager — and runs as uid 65532. Deploy is "copy one binary", not "ship
a Python runtime + virtualenv + gunicorn".

## Build and run

```console
$ delonix stack up        # or `docker compose up` — builds the Delonixfile
```

The generated `compose.yaml` builds from the `Delonixfile` and maps `8080`.
Optional backing services (the PostgreSQL from Part 3, a broker) live in
`compose/*.yaml` and are opted in explicitly.

## Health and readiness

The service exposes `/health/live` and `/health/ready` over a
`Starting → Ready → Draining` lifecycle. Distroless has no shell for a CMD
healthcheck, so Kubernetes uses an httpGet readiness probe on `/health/ready`. On
`SIGTERM` the service drains in-flight requests before exiting — a rollout is
graceful.

## Configuration

Everything is environment (12-factor): `SEIVA_HTTP_ADDR`, `APP_ENV`, the OIDC
variables from Part 4, and `DELONIX_ORM_DSN` for the data layer from Part 3. The
platform injects secrets; nothing is baked into the image. See
[Configuration](../../topics/configuration.md).

## You have a service

From `seiva project new` to a distroless, authenticated, database-backed,
self-documenting polls service — the same app as the Django tutorial, as one
compiled binary. Next: the [topic guides](../../topics/index.md) for depth, or the
[competitive landscape](../../competitive-landscape.md) for where Seiva stands
against Django and FastAPI.

---

*Part of the [Seiva documentation](../../index.md).*
