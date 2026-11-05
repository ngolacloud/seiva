#!/usr/bin/env bash
#
# Reproducible benchmark harness: Seiva (Rust) vs FastAPI vs Django on the
# SAME trivial workload. See README.md for the methodology and caveats.
#
# For each framework it: builds/starts a production-ish server, waits for it
# to answer, WARMS UP, runs `oha` against two endpoints (GET /health and
# POST /v1/notes), records throughput + latency percentiles, then stops the
# server and frees the port. Results are written to results/<timestamp>/.
#
# Config (override via environment):
#   CONNECTIONS   concurrent connections oha opens         (default 50)
#   DURATION      measured seconds per endpoint            (default 20)
#   WARMUP        warmup seconds per endpoint (discarded)  (default 5)
#   WORKERS       Python server worker processes           (default: nproc)
#                 Seiva uses the tokio multi-thread runtime (all cores) and
#                 ignores this.
#   SEIVA_PORT / FASTAPI_PORT / DJANGO_PORT  listen ports  (18080/81/82)
#   ONLY          comma list to restrict frameworks, e.g. ONLY=seiva,fastapi
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"

CONNECTIONS="${CONNECTIONS:-50}"
DURATION="${DURATION:-20}"
WARMUP="${WARMUP:-5}"
WORKERS="${WORKERS:-$(nproc)}"
SEIVA_PORT="${SEIVA_PORT:-18080}"
FASTAPI_PORT="${FASTAPI_PORT:-18081}"
DJANGO_PORT="${DJANGO_PORT:-18082}"
ONLY="${ONLY:-seiva,fastapi,django}"

PY="$HERE/.venv/bin/python"
OHA="$(command -v oha || true)"
[ -z "$OHA" ] && [ -x "$HOME/.cargo/bin/oha" ] && OHA="$HOME/.cargo/bin/oha"
SEIVA_BIN="$HERE/notesbench/target/release/notesbench-api"

STAMP="$(date +%Y%m%d-%H%M%S)"
OUTDIR="$HERE/results/$STAMP"
mkdir -p "$OUTDIR"
TSV="$OUTDIR/raw.tsv"
printf 'framework\tendpoint\trps\tp50_ms\tp95_ms\tp99_ms\tsuccess\ttotal\n' > "$TSV"

POST_BODY='{"title":"benchmark"}'
SERVER_PID=""

log()  { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
want() { [[ ",$ONLY," == *",$1,"* ]]; }

cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  for p in "$SEIVA_PORT" "$FASTAPI_PORT" "$DJANGO_PORT"; do
    fuser -k "${p}/tcp" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

free_port() { fuser -k "${1}/tcp" 2>/dev/null || true; sleep 0.5 || true; }

wait_up() {  # wait_up <url>  -- poll until it answers (2xx/4xx), max ~15s
  local url="$1" i
  for i in $(seq 1 150); do
    if curl -s -o /dev/null --max-time 1 "$url"; then return 0; fi
    sleep 0.1
  done
  echo "server did not come up at $url" >&2; return 1
}

# run_endpoint <framework> <label> <url> <method> [body]
run_endpoint() {
  local fw="$1" label="$2" url="$3" method="$4" body="${5:-}"
  local args=(-c "$CONNECTIONS" --no-tui --disable-compression --output-format json "$url")
  if [ "$method" = POST ]; then
    args=(-m POST -d "$body" -T application/json "${args[@]}")
  fi
  # Warmup (discarded).
  "$OHA" -z "${WARMUP}s" "${args[@]}" >/dev/null 2>&1 || true
  # Measured run.
  local json="$OUTDIR/${fw}_${label}.json"
  "$OHA" -z "${DURATION}s" "${args[@]}" > "$json" 2>/dev/null
  "$PY" - "$fw" "$label" "$json" "$TSV" <<'PYEOF'
import json, sys
fw, label, path, tsv = sys.argv[1:5]
d = json.load(open(path))
s = d["summary"]
pct = d["latencyPercentiles"]
rps = s["requestsPerSec"]
# NOTE: summary["total"] is ELAPSED SECONDS, not a request count. Derive the
# real counts from the status-code distribution (+ transport errors).
status = {int(k): int(v) for k, v in d.get("statusCodeDistribution", {}).items()}
errors = sum(int(v) for v in d.get("errorDistribution", {}).values())
success = sum(v for k, v in status.items() if 200 <= k < 300)
total = sum(status.values()) + errors
# oha latency percentiles are in SECONDS -> milliseconds.
p50 = pct["p50"] * 1000
p95 = pct["p95"] * 1000
p99 = pct["p99"] * 1000
with open(tsv, "a") as f:
    f.write(f"{fw}\t{label}\t{rps:.0f}\t{p50:.3f}\t{p95:.3f}\t{p99:.3f}\t{success}\t{total}\n")
print(f"  {fw:8} {label:12} {rps:>12,.0f} rps   "
      f"p50 {p50:7.3f} ms  p95 {p95:7.3f} ms  p99 {p99:7.3f} ms  "
      f"({success}/{total} 2xx)")
PYEOF
}

bench_framework() {  # <framework> <host:port> <health_url>
  local fw="$1" base="$2" health="$3"
  wait_up "$health"
  log "warmup + measure: $fw"
  run_endpoint "$fw" health "$health" GET
  run_endpoint "$fw" notes "http://$base/v1/notes" POST "$POST_BODY"
}

### Preconditions ########################################################
[ -n "$OHA" ] || { echo "oha not found (expected on PATH or ~/.cargo/bin)"; exit 1; }
[ -x "$PY" ] || { echo "python venv missing; creating..."; python3 -m venv "$HERE/.venv"; }
log "ensuring python deps"
"$HERE/.venv/bin/pip" install --quiet -r "$HERE/fastapi_app/requirements.txt" "django==5.1.*"

log "building Seiva release binary"
( cd "$HERE/notesbench" && CARGO_TARGET_DIR=target cargo build --release -p notesbench-api )

cat <<CFG

Profile: connections=$CONNECTIONS duration=${DURATION}s warmup=${WARMUP}s python_workers=$WORKERS
Host:    $(nproc) CPUs / $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | sed 's/^ //')
Load:    oha $("$OHA" --version)
CFG

### Seiva ################################################################
# Seiva's tokio listener does not set SO_REUSEADDR, so a port left in
# TIME_WAIT by an earlier aborted run can fail the bind. Retry on a bumped
# port until it actually answers.
if want seiva; then
  started=0
  for off in 0 1 2 3 4; do
    port=$((SEIVA_PORT + off))
    free_port "$port"
    SEIVA_HTTP_ADDR="127.0.0.1:$port" "$SEIVA_BIN" >"$OUTDIR/seiva.server.log" 2>&1 &
    SERVER_PID=$!
    if wait_up "http://127.0.0.1:$port/health/live"; then
      bench_framework seiva "127.0.0.1:$port" "http://127.0.0.1:$port/health/live"
      started=1
      kill "$SERVER_PID" 2>/dev/null || true; wait 2>/dev/null || true; SERVER_PID=""
      break
    fi
    echo "  seiva failed to bind :$port, retrying on next port" >&2
    kill "$SERVER_PID" 2>/dev/null || true; wait 2>/dev/null || true; SERVER_PID=""
  done
  [ "$started" = 1 ] || { echo "seiva never came up"; exit 1; }
fi

### FastAPI (uvicorn, N workers) #########################################
if want fastapi; then
  free_port "$FASTAPI_PORT"
  "$PY" -m uvicorn --app-dir "$HERE/fastapi_app" main:app \
      --host 127.0.0.1 --port "$FASTAPI_PORT" --workers "$WORKERS" \
      --no-access-log --log-level warning >"$OUTDIR/fastapi.server.log" 2>&1 &
  SERVER_PID=$!
  bench_framework fastapi "127.0.0.1:$FASTAPI_PORT" "http://127.0.0.1:$FASTAPI_PORT/health"
  kill "$SERVER_PID" 2>/dev/null || true; wait 2>/dev/null || true; SERVER_PID=""
fi

### Django (gunicorn WSGI, N sync workers) ###############################
if want django; then
  free_port "$DJANGO_PORT"
  "$HERE/.venv/bin/gunicorn" --pythonpath "$HERE/django_app" app:application \
      --bind "127.0.0.1:$DJANGO_PORT" --workers "$WORKERS" \
      --access-logfile /dev/null --error-logfile - >"$OUTDIR/django.server.log" 2>&1 &
  SERVER_PID=$!
  bench_framework django "127.0.0.1:$DJANGO_PORT" "http://127.0.0.1:$DJANGO_PORT/health"
  kill "$SERVER_PID" 2>/dev/null || true; wait 2>/dev/null || true; SERVER_PID=""
fi

### Results table ########################################################
log "writing results table"
"$PY" - "$TSV" "$OUTDIR" "$CONNECTIONS" "$DURATION" "$WARMUP" "$WORKERS" <<'PYEOF'
import sys
tsv, outdir, c, dur, warm, workers = sys.argv[1:7]
import platform, subprocess
rows = [l.rstrip("\n").split("\t") for l in open(tsv)][1:]
try:
    cpu = subprocess.check_output(
        "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2",
        shell=True, text=True).strip()
except Exception:
    cpu = platform.processor()
ncpu = __import__("os").cpu_count()

def table(endpoint):
    sel = [r for r in rows if r[1] == endpoint]
    out = ["| Framework | Requests/sec | p50 (ms) | p95 (ms) | p99 (ms) | 2xx / total |",
           "|---|---:|---:|---:|---:|---:|"]
    for r in sel:
        fw, _lab, rps, p50, p95, p99, succ, tot = r
        out.append(f"| {fw} | {int(rps):,} | {p50} | {p95} | {p99} | {succ}/{tot} |")
    return "\n".join(out)

md = f"""# Benchmark results

- Host: {ncpu} logical CPUs /{cpu}
- Load generator: oha, {c} connections, {dur}s measured ({warm}s warmup discarded), HTTP/1.1 keep-alive, loopback
- Python servers: {workers} worker processes (FastAPI = uvicorn; Django = gunicorn WSGI). Seiva = tokio multi-thread runtime.
- Seiva built `--release`; rate limiter disabled for the run (see README).

These numbers are **environment-dependent** and were measured on a shared
developer machine over loopback (the load generator competes with the server
for CPU). Treat them as directional, not absolute. Re-run with `./run.sh`.

## GET /health  (trivial liveness; no body)

{table("health")}

## POST /v1/notes  (validate non-empty title, assign id, echo JSON)

{table("notes")}
"""
open(f"{outdir}/results.md", "w").write(md)
print(md)
print(f"\nSaved: {outdir}/results.md  (raw oha JSON + server logs alongside)")
PYEOF

log "done"
