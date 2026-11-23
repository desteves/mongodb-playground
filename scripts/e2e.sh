#!/usr/bin/env bash
# Browser tests (Playwright) against a real stack: an Atlas CLI local deployment, the API serving the
# built web app, and Chromium. The deployment is deleted on exit.
#   first time:  (cd web && npm install && npx playwright install chromium)
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=mdbtest
PORT=6310
API_PORT=${API_PORT:-$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')}
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export MONGODB_URI="mongodb://localhost:${PORT}/?directConnection=true"

API_PID=""
cleanup() {
  [ -n "$API_PID" ] && kill "$API_PID" 2>/dev/null || true
  atlas local delete "$NAME" --force || true
}
trap cleanup EXIT

atlas local delete "$NAME" --force >/dev/null 2>&1 || true
atlas local setup "$NAME" --port "$PORT" --force

(cd web && npm run build)
cargo build -p playground-api
PORT="$API_PORT" STATS_DISABLED=1 SHARES_PER_CLIENT_PER_DAY=100000 RATE_BASIC_BURST=100000 RATE_BASIC_PER_SEC=100000 RATE_VERIFIED_BURST=100000 RATE_VERIFIED_PER_SEC=100000 ./target/debug/api &
API_PID=$!
for _ in $(seq 1 30); do
  kill -0 "$API_PID" 2>/dev/null || { echo "the API exited before it was healthy" >&2; exit 1; }
  curl -fs "http://localhost:${API_PORT}/healthz" >/dev/null && break
  sleep 1
done

(cd web && BASE_URL="http://localhost:${API_PORT}" npx playwright test "$@")
