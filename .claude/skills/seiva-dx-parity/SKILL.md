---
name: seiva-dx-parity
description: Run the FastAPI developer-experience parity checklist when changing seiva-http, seiva-openapi or the CLI. Fails if a FastAPI developer's "first day" gets worse.
---

# Seiva ↔ FastAPI DX parity

A developer who knows FastAPI should orient in a day. Run this checklist when a
change touches `seiva-http`, `seiva-openapi` or `seiva-cli`. The baseline lives
in `docs/first-day.md` — update it when the status changes.

## Checklist (each: still true? did this change improve or regress it?)

| Habit (FastAPI) | Seiva today | Must not regress |
| --- | --- | --- |
| Route decorator | `#[seiva::facade]` method → route generated from the descriptor | a facade method still maps to a route with no hand-written router |
| `pydantic` body | `Valid<T>` extractor → `422` RFC 9457 (`seiva-http/src/valid.rs`) | bodies stay typed + validated |
| `/docs` (Swagger UI) | `GET /docs` serves Scalar over `/openapi.json` (`seiva-http/src/docs.rs`) | the docs page still loads and points at the service's own spec |
| `/openapi.json` | generated from every linked facade (`registered_facades`) | a mounted module still appears |
| `fastapi dev --reload` | `seiva start` (cargo-watch hot-reload else plain run) | `start` still runs the project in dev |
| `Depends()` | ports injected into the application service (documented in `docs/architecture.md`) | DI stays explicit and documented |
| Error model | RFC 9457 `Problem` (stricter than FastAPI default) | errors stay `problem+json` with a request id |

## Known gaps (do not let them widen)

- No reference auth middleware that populates `Principal` yet (JWT/session).
- `Valid<T>` carries a single message, not a per-field map.
- The standalone CLI documents only linked facades (it is not compiled against a
  user's project) — the runtime `/openapi.json` is the complete one.

## Output

The table with a ✓/✗ per row and the change's delta. If any row regresses, FAIL
and name the regression. If the change adds a first-day improvement, note it and
update `docs/first-day.md`.
