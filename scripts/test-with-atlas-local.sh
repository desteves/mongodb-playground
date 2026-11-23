#!/usr/bin/env bash
# Run the whole test suite against a throwaway Atlas CLI local deployment, then delete it.
#   1. cargo test (unit + integration, with MONGODB_URI set)
#   2. start the API and run scripts/smoke.py against it
# The deployment is deleted on exit, even if something fails.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=mdbtest
PORT=6310          # MongoDB
# A free port, so a dev server already on 8081 (scripts/dev.sh, the preview pane) cannot be mistaken for this run.
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

echo "== cargo test =="
cargo test --workspace --exclude playground-desktop

echo "== starting the API on :${API_PORT} =="
cargo build -p playground-api
echo "== seeding the prebuilt example datasets (a search and a vector index) =="
./target/debug/api seed-examples
# The smoke test sends hundreds of requests in a burst: lift the rate limits. The sample-data feature is on so
# it is exercised too; production leaves it off.
PORT="$API_PORT" FEATURE_SAMPLE_DATA=1 SHARES_PER_CLIENT_PER_DAY=100000 RATE_BASIC_BURST=100000 RATE_BASIC_PER_SEC=100000 RATE_VERIFIED_BURST=100000 RATE_VERIFIED_PER_SEC=100000 ./target/debug/api &
API_PID=$!
for _ in $(seq 1 30); do
  kill -0 "$API_PID" 2>/dev/null || { echo "the API exited before it was healthy" >&2; exit 1; }
  curl -fs "http://localhost:${API_PORT}/healthz" >/dev/null && break
  sleep 1
done

echo "== smoke test =="
python3 scripts/smoke.py "http://localhost:${API_PORT}"
