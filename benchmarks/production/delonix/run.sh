#!/usr/bin/env bash
# Fully containerised, load-balanced multi-replica proof, run by `delonix` (no
# Docker daemon). Brings up PostgreSQL + Redis + two Catalog replicas + an HAProxy
# load balancer, then proves two things the single-host benchmark could not:
#
#   A. SHARED BUDGET — with a small per-client rate limit, the TOTAL 2xx across
#      both replicas is ~ONE budget (not 2x), because the distributed Redis
#      limiter (ADR 0015) counts in a shared store. A per-process limiter behind a
#      round-robin LB would admit ~2x (each replica its own counter).
#   B. THROUGHPUT — with the limit raised, the authed + tenant-scoped DB path
#      sustained through the containerised LB (p50/p95/p99, 2xx).
#
# Needs: delonix on PATH, oha, and the host toolchain to build the release binary.
# Not in CI. Honest: single host, one Docker-bridge network; it proves the
# cross-replica budget and the containerised path, not a capacity SLA.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
PROJECT="seiva-prodlb"
LB="http://127.0.0.1:8080"
TOKEN="clerk-token"
auth=(-H "authorization: Bearer $TOKEN")
OHA="$(command -v oha || true)"; [ -z "$OHA" ] && [ -x "$HOME/.cargo/bin/oha" ] && OHA="$HOME/.cargo/bin/oha"
[ -n "$OHA" ] || { echo "oha not found"; exit 1; }
command -v delonix >/dev/null || { echo "delonix not found"; exit 1; }

echo "==> building the release binary (host) and staging it into the build context"
( cd "$ROOT" && cargo build -q --release --manifest-path apps/admin-data-e2e/Cargo.toml --bin admin-data-e2e )
cp "$ROOT/apps/admin-data-e2e/target/release/admin-data-e2e" "$HERE/app/admin-data-e2e"

cd "$HERE"
RENDERED="docker-compose.rendered.yml"
down(){ delonix compose down -p "$PROJECT" >/dev/null 2>&1 || true; }
up(){ # $1 = per-client rate limit (req/min) — delonix compose does not interpolate
      # ${VAR}, so render the limit into a throwaway file and point `-f` at it.
  sed "s/__RATE_LIMIT__/$1/g" docker-compose.yml > "$RENDERED"
  delonix compose up -f "$RENDERED" -p "$PROJECT" -d
  echo "   waiting for the load balancer to report all replicas ready"
  for _ in $(seq 1 120); do
    code=$(curl -s -o /dev/null -w '%{http_code}' "$LB/health/ready" || true)
    [ "$code" = "200" ] && { echo "   LB ready"; return 0; }
    sleep 1
  done
  echo "LB never became ready"; delonix compose ps -p "$PROJECT"; delonix compose logs -p "$PROJECT" lb | tail -20; exit 1
}
trap down EXIT

seed(){ curl -s "${auth[@]}" -H 'content-type: application/json' \
  -d '{"name":"seed","price":1,"cost":1}' "$LB/v1/catalogs" >/dev/null || true; }

echo
echo "### Proof A — ONE rate budget across two replicas (distributed Redis limiter)"
BUDGET=120
down; up "$BUDGET"; seed
# Send a fixed, large burst through the LB; both replicas serve it round-robin but
# share one Redis counter, so the admitted 2xx total must be ~BUDGET, not ~2*BUDGET.
"$OHA" -n 4000 -c 50 --no-tui --output-format json "${auth[@]}" "$LB/v1/catalogs" > "$HERE/budget.json"
python3 - "$HERE/budget.json" "$BUDGET" <<'PY'
import json,sys
d=json.load(open(sys.argv[1])); budget=int(sys.argv[2])
codes=d.get("statusCodeDistribution") or {}
ok=codes.get("200",0); limited=codes.get("429",0)
print(f"  budget={budget}/min | admitted 2xx={ok} | 429={limited} | codes {codes}")
# One shared budget: admitted is close to BUDGET and clearly below 2*BUDGET.
assert ok <= budget*1.6, f"admitted {ok} exceeds ~budget {budget}: budget not shared across replicas!"
assert ok >= budget*0.5, f"admitted {ok} far below budget {budget}: unexpected"
assert limited > 0, "expected the limiter to reject the rest of the burst"
print(f"  PASS: the two replicas enforced ONE budget (~{budget}), not ~{2*budget}")
PY

echo
echo "### Proof B — throughput through the containerised LB (limit raised)"
down; up 100000000; seed
for name in lb_read lb_read2; do
  "$OHA" -z 10s -c 50 --no-tui --output-format json "${auth[@]}" "$LB/v1/catalogs" > "$HERE/$name.json"
done
"$OHA" -z 10s -c 50 --no-tui --output-format json -m POST "${auth[@]}" \
  -H 'content-type: application/json' -d '{"name":"load","price":9,"cost":3}' "$LB/v1/catalogs" > "$HERE/lb_write.json"
for name in lb_read lb_read2 lb_write; do
python3 - "$HERE/$name.json" "$name" <<'PY'
import json,sys
d=json.load(open(sys.argv[1])); s=d["summary"]; p=d.get("latencyPercentiles") or {}
codes=d.get("statusCodeDistribution") or {}; ms=lambda x: f"{(x or 0)*1000:.2f}ms"
ok=codes.get("200",0)+codes.get("201",0)
print(f"  {sys.argv[2]}: {s['requestsPerSec']:.0f} req/s | p50 {ms(p.get('p50'))} p95 {ms(p.get('p95'))} p99 {ms(p.get('p99'))} | {ok} 2xx, codes {codes}")
PY
done

echo
echo "==> all proofs passed. Tearing the stack down."
