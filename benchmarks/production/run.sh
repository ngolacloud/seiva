#!/usr/bin/env bash
# Production-PATH load benchmark (Seiva §9/§11): measures the SECURE path —
# authentication ON, over a real PostgreSQL via DORM — not the overhead
# microbenchmark (which turns auth off). Runs TWO replicas of the `admin-data-e2e`
# service (two processes sharing one PostgreSQL — the shared-state replica model)
# and drives `oha` against them. Reports p50/p95/p99, throughput and status codes.
#
# Two scenarios, both honest:
#   1. PROTECTED  — the secure default (300 req/min per IP). Shows the limiter
#      shedding a 50-connection flood as cheap 429s: the DoS protection at work.
#   2. WORK PATH  — rate limit raised (ADMIN_DATA_E2E_RATE_LIMIT) so the limiter
#      does not bind, measuring the real cost of auth + tenant-scoped DB read/write.
#
# Honesty: single-host numbers, environment-dependent; two processes model two
# replicas for the shared PostgreSQL. A containerised, orchestrated deployment
# (delonix) with a load balancer and the distributed rate-limit/idempotency is the
# production form — this proves the secure path's cost, not a capacity SLA.
#
# Needs: `oha` on PATH (or ~/.cargo/bin), a reachable PostgreSQL. Not in CI.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OHA="$(command -v oha || true)"; [ -z "$OHA" ] && [ -x "$HOME/.cargo/bin/oha" ] && OHA="$HOME/.cargo/bin/oha"
[ -n "$OHA" ] || { echo "oha not found (cargo install oha)"; exit 1; }
DSN="${PROD_BENCH_DSN:-host=/var/run/postgresql dbname=seiva_prod_bench}"
TOKEN="clerk-token"
DUR="${PROD_BENCH_DURATION:-10s}"
CONN="${PROD_BENCH_CONNECTIONS:-50}"
WORK_RPM="${PROD_BENCH_WORK_RPM:-100000000}"  # effectively unbounded for the work-path run
OUT="$HERE/results"; mkdir -p "$OUT"
auth=(-H "authorization: Bearer $TOKEN")

echo "==> building admin-data-e2e (release)"
( cd "$ROOT" && cargo build -q --release --manifest-path apps/admin-data-e2e/Cargo.toml --bin admin-data-e2e )
BIN="$ROOT/apps/admin-data-e2e/target/release/admin-data-e2e"

wait_ready(){ for _ in $(seq 1 150); do curl -s -o /dev/null "http://127.0.0.1:$1/admin" && return 0; sleep 0.2; done; echo "replica on $1 never came up"; exit 1; }

start_replicas(){ # $1 = rate-limit RPM (empty → secure default)
  local rpm="${1:-}"
  echo "==> starting two replicas (shared PostgreSQL)${rpm:+, rate limit ${rpm}/min}"
  # Staggered: R1 creates the table (idempotent DDL) and goes ready before R2
  # starts, so the two never race on the sequence creation.
  local rl=(); [ -n "$rpm" ] && rl=(ADMIN_DATA_E2E_RATE_LIMIT="$rpm")
  env ADMIN_DATA_E2E_ADDR=127.0.0.1:8091 ADMIN_DATA_E2E_DSN="$DSN" "${rl[@]}" "$BIN" & R1=$!
  wait_ready 8091
  env ADMIN_DATA_E2E_ADDR=127.0.0.1:8092 ADMIN_DATA_E2E_DSN="$DSN" "${rl[@]}" "$BIN" & R2=$!
  wait_ready 8092
  trap 'kill "$R1" "$R2" 2>/dev/null || true' EXIT
}
stop_replicas(){ kill "$R1" "$R2" 2>/dev/null || true; wait "$R1" "$R2" 2>/dev/null || true; trap - EXIT; sleep 0.5; }

report(){ python3 - "$OUT/$1.json" "$1" <<'PY'
import json,sys
d=json.load(open(sys.argv[1])); s=d["summary"]; p=d.get("latencyPercentiles") or {}
codes=d.get("statusCodeDistribution") or {}
errs=d.get("errorDistribution") or {}
ms=lambda x: f"{(x or 0)*1000:.2f}ms"
n_codes=sum(codes.values()); n_err=sum(errs.values())
ok=codes.get("200",0)+codes.get("201",0)
print(f"  {sys.argv[2]}: {s['requestsPerSec']:.0f} req/s | "
      f"p50 {ms(p.get('p50'))} p95 {ms(p.get('p95'))} p99 {ms(p.get('p99'))} | "
      f"{n_codes+n_err} reqs ({ok} 2xx), codes {codes}" + (f", errors {n_err}" if n_err else ""))
PY
}

run(){ # name url method [body]
  local name="$1" url="$2" method="$3" body="${4:-}"
  echo "==> $name ($method $url, ${CONN} conns, ${DUR})"
  if [ "$method" = "POST" ]; then
    "$OHA" -z "$DUR" -c "$CONN" --no-tui --output-format json -m POST \
      "${auth[@]}" -H 'content-type: application/json' -d "$body" "$url" > "$OUT/$name.json"
  else
    "$OHA" -z "$DUR" -c "$CONN" --no-tui --output-format json "${auth[@]}" "$url" > "$OUT/$name.json"
  fi
  report "$name"
}

echo
echo "### Scenario 1: PROTECTED (secure default, 300 req/min per IP)"
start_replicas ""
curl -s "${auth[@]}" -H 'content-type: application/json' \
  -d '{"name":"seed","price":1,"cost":1}' "http://127.0.0.1:8091/v1/catalogs" >/dev/null || true
run protected_read "http://127.0.0.1:8091/v1/catalogs" GET
stop_replicas

echo
echo "### Scenario 2: WORK PATH (rate limit raised to ${WORK_RPM}/min — measures auth+DB)"
start_replicas "$WORK_RPM"
curl -s "${auth[@]}" -H 'content-type: application/json' \
  -d '{"name":"seed","price":1,"cost":1}' "http://127.0.0.1:8091/v1/catalogs" >/dev/null || true
run work_read_r1  "http://127.0.0.1:8091/v1/catalogs" GET
run work_read_r2  "http://127.0.0.1:8092/v1/catalogs" GET
run work_write_r1 "http://127.0.0.1:8091/v1/catalogs" POST '{"name":"load","price":9,"cost":3}'
stop_replicas

echo
echo "==> done. Raw oha JSON in $OUT/"
