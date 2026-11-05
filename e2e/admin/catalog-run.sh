#!/usr/bin/env bash
# Runs the console ROW-CRUD browser E2E over the real DORM-backed `Catalog`
# module (admin-data-e2e) on PostgreSQL. Needs Node, Google Chrome and a reachable
# PostgreSQL (DSN via ADMIN_DATA_E2E_DSN; defaults to a local peer-auth socket).
# On-demand, not part of `cargo test`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ADDR="${ADMIN_DATA_E2E_ADDR:-127.0.0.1:8098}"
DSN="${ADMIN_DATA_E2E_DSN:-host=/var/run/postgresql dbname=seiva_catalog_e2e}"
export CATALOG_E2E_BASE="http://${ADDR}"

echo "==> building admin-data-e2e"
( cd "$ROOT" && cargo build -q --manifest-path apps/admin-data-e2e/Cargo.toml --bin admin-data-e2e )

echo "==> installing Playwright (uses system Chrome)"
( cd "$HERE" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --silent --no-audit --no-fund )

echo "==> starting server on ${ADDR} (DSN: ${DSN})"
ADMIN_DATA_E2E_ADDR="$ADDR" ADMIN_DATA_E2E_DSN="$DSN" \
  "$ROOT/apps/admin-data-e2e/target/debug/admin-data-e2e" &
SRV=$!
trap 'kill "$SRV" 2>/dev/null || true' EXIT

for _ in $(seq 1 100); do
  curl -s -o /dev/null "http://${ADDR}/admin" && break || sleep 0.2
done

echo "==> running the row-CRUD browser E2E"
( cd "$HERE" && node catalog.mjs )
