"""Minimal FastAPI service with the SAME endpoints as the Seiva app.

Endpoints (kept logically identical across all three frameworks):
  GET  /health      -> 200 {"status": "ok"}        (trivial liveness)
  POST /v1/notes    -> validate non-empty `title`, assign a monotonic id,
                       echo back {"id": <n>, "title": <title>}

No database: the "repository" is an in-process atomic counter, exactly like
the Seiva scaffold's in-memory repository + counter id-generator. This keeps
the benchmark measuring the FRAMEWORK, not I/O.
"""

from itertools import count
from threading import Lock

from fastapi import FastAPI
from fastapi.responses import JSONResponse
from pydantic import BaseModel, field_validator

app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)

# Monotonic id source shared across worker threads of a single process.
# (Each uvicorn/gunicorn worker is its own process with its own counter, which
# is fine -- ids need not be globally unique for this throughput benchmark, and
# the Seiva counter is likewise process-local.)
_counter = count(1)
_lock = Lock()


class CreateRecord(BaseModel):
    title: str

    @field_validator("title")
    @classmethod
    def title_not_blank(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("title must not be empty")
        return v


@app.get("/health")
async def health() -> JSONResponse:
    return JSONResponse({"status": "ok"})


@app.post("/v1/notes", status_code=201)
async def create_note(cmd: CreateRecord) -> JSONResponse:
    with _lock:
        new_id = next(_counter)
    return JSONResponse({"id": new_id, "title": cmd.title}, status_code=201)
