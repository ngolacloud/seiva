# How to deploy a distroless image

A generated project is cloud-native out of the box — the recipe is mostly "build
the image it already has".

1. **Build the image.** The generated `Delonixfile` is a multi-stage, distroless,
   non-root build:

   ```console
   $ delonix stack up        # or `docker compose up` — builds the Delonixfile
   ```

   The final image carries only the binary and its runtime libraries (no shell, no
   package manager) and runs as uid 65532.

2. **Configure via environment.** Nothing is baked in: set `SEIVA_HTTP_ADDR`
   (the image defaults to `0.0.0.0:8080`), `APP_ENV=production`, the OIDC
   variables, and `DELONIX_ORM_DSN`; the platform injects secrets. See
   [Settings](../reference/settings.md).

3. **Probe readiness with httpGet.** Distroless has no shell for a CMD
   healthcheck, so use a Kubernetes httpGet readiness probe on `/health/ready`
   (and a liveness probe on `/health/live`). On `SIGTERM` the service drains
   in-flight requests, so a rolling update is graceful.

## Opt-in backing services

The generated `compose.yaml` runs the app; a database or broker lives in
`compose/*.yaml` and is opted in explicitly:

```console
$ delonix stack up -f compose.yaml -f compose/postgres.yaml
```

## Related

- [Tutorial part 6](../getting-started/shop/part-7-monolith-to-microservices.md) · [Performance & cloud-native](../topics/performance.md)

---

*Part of the [Seiva documentation](../index.md).*
