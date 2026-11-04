---
name: seiva-cloud-native-check
description: Prove a Seiva service is cloud-native-ready before a release or any change to the scaffold/Delonixfile. Builds a distroless image, measures its size, confirms it has no shell and runs non-root, and that it answers /health/ready.
---

# Seiva cloud-native check

The "ready" bar (audit Part D): a built image is **distroless, multi-stage,
non-root**, and the container answers `200` on `/health/ready` with **no manual
configuration**. Logs are structured JSON on stdout.

## Run

1. **Scaffold shape.** Confirm `scaffold::project_files` still emits `Delonixfile`,
   `compose.yaml`, `.dockerignore` and a real `apps/<name>-api/main.rs` (uses
   `serve_router` + `observability::init`, not a `println!` stub). The test
   `project_is_cloud_native_ready` guards this — run
   `cargo test -p delonix-seiva project_is_cloud_native_ready`.
2. **Build a real image.** A generated project does not yet compile standalone
   (framework crates are path deps pending publish), so prove the recipe against
   the real `seiva-server`: build the Delonixfile recipe with `docker build`
   (context = a clean `git archive HEAD`), e.g.
   `FROM rust:1-slim AS builder … FROM gcr.io/distroless/cc-debian12:nonroot`.
3. **Measure and assert** on the running container:
   - `docker images … --format '{{.Size}}'` — small (tens of MB, not hundreds).
   - `docker exec <c> /bin/sh` **fails** (no shell → distroless).
   - `docker inspect --format '{{.Config.User}}'` is `nonroot`.
   - `curl /health/ready` → `200`, `/health/live` → `200`, `/docs` → `200`.
   - `/openapi.json` lists every mounted facade (discovery).
   - `docker logs` lines are JSON (`{"timestamp":…,"level":…}`).
4. **compose.** `delonix stack up` (or `docker compose up`) builds from the
   Delonixfile and maps the port.

## Output

A short report: image size, distroless (shell absent), non-root, the health/docs
status codes, the discovered facades, and a sample JSON log line. Note any
regression against the last recorded proof in `docs/progress.md` (S3).
