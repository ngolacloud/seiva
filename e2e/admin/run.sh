#!/usr/bin/env bash
# Runs the admin browser E2E (§10): build + start the `admin-e2e` server, drive a
# headless system Chrome through Playwright, then stop the server. Needs Node and
# Google Chrome on the host (no chromium is downloaded — Playwright uses the
# system Chrome via `channel: "chrome"`). On-demand, not part of `cargo test`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ADDR="${ADMIN_E2E_ADDR:-127.0.0.1:8099}"
export ADMIN_E2E_BASE="http://${ADDR}"

echo "==> building admin-e2e"
( cd "$ROOT" && cargo build -q -p admin-e2e )

echo "==> installing Playwright (no browser download; uses system Chrome)"
( cd "$HERE" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --silent --no-audit --no-fund )

echo "==> starting server on ${ADDR}"
ADMIN_E2E_ADDR="$ADDR" "$ROOT/target/debug/admin-e2e" &
SRV=$!
trap 'kill "$SRV" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  curl -s -o /dev/null "http://${ADDR}/admin" && break || sleep 0.2
done

echo "==> running the browser E2E"
( cd "$HERE" && node admin.mjs )
