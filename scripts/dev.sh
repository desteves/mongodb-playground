#!/usr/bin/env bash
# Run the API (which also serves the built web app) against the local Atlas CLI test deployment.
#   atlas local setup mdbtest --port 6310 --force      # once, in your terminal
#   scripts/dev.sh                                     # then open http://localhost:8081
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export MONGODB_URI="${MONGODB_URI:-mongodb://localhost:6310/?directConnection=true}"
export PORT="${PORT:-8081}"
export STATS_DISABLED="${STATS_DISABLED:-1}"
[ -f web/dist/index.html ] || (cd web && npm install --no-audit --no-fund && npm run build)
exec cargo run -p playground-api
