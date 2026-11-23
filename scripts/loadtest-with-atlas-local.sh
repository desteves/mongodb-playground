#!/usr/bin/env bash
# Load test then abuse test, against an Atlas CLI local deployment. The deployment is deleted on exit.
#   scripts/loadtest-with-atlas-local.sh [duration seconds] [concurrency]
# Afterwards it checks that no pg_* database was left behind (needs mongosh; skipped if it is not installed).
set -euo pipefail
cd "$(dirname "$0")/.."
DURATION=${1:-20}
CONCURRENCY=${2:-8}

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

cargo build --release -p playground-api
PORT="$API_PORT" STATS_DISABLED=1 SHARES_PER_CLIENT_PER_DAY=100000 RATE_BASIC_BURST=100000 RATE_BASIC_PER_SEC=100000 RATE_VERIFIED_BURST=100000 RATE_VERIFIED_PER_SEC=100000 ./target/release/api &
API_PID=$!
for _ in $(seq 1 30); do
  kill -0 "$API_PID" 2>/dev/null || { echo "the API exited before it was healthy" >&2; exit 1; }
  curl -fs "http://localhost:${API_PORT}/healthz" >/dev/null && break
  sleep 1
done

BASE="http://localhost:${API_PORT}"
echo "== load: ${DURATION}s, ${CONCURRENCY} workers =="
python3 scripts/loadtest.py "$BASE" --duration "$DURATION" --concurrency "$CONCURRENCY"
echo "== abuse =="
python3 scripts/loadtest.py "$BASE" --abuse

if command -v mongosh >/dev/null 2>&1; then
  sleep 2
  LEFT=$(mongosh "$MONGODB_URI" --quiet --eval 'db.adminCommand({listDatabases: 1, nameOnly: true}).databases.filter(d => d.name.startsWith("pg_")).length')
  echo "leftover pg_* databases: ${LEFT}"
  [ "$LEFT" = "0" ] || { echo "FAIL: playground databases were left behind" >&2; exit 1; }
else
  echo "(mongosh not installed: skipped the leftover-database check)"
fi
