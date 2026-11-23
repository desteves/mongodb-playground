#!/usr/bin/env bash
# Run the desktop app (Tauri) in development against a local Atlas deployment, which has Atlas Search and Vector Search.
#   scripts/dev-desktop.sh              start (or reuse) the local deployment, install web dependencies, open the app
#   scripts/dev-desktop.sh --no-db      skip the deployment; use your own MongoDB (default mongodb://localhost:27017)
#   scripts/dev-desktop.sh --delete     delete the local deployment and stop
# The deployment listens on 27017 (DESKTOP_DB_PORT), the port the app connects to by default, so nothing needs configuring.
# Needs: the Atlas CLI, Docker (or Podman) for the deployment, Node, and Rust. On Linux, Tauri's system libraries too:
# https://v2.tauri.app/start/prerequisites/
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=playground-desktop
PORT=${DESKTOP_DB_PORT:-27017}
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1 ($2)" >&2; exit 1; }; }
need node "https://nodejs.org"
need npm "https://nodejs.org"
need cargo "https://rustup.rs"

case "${1:-}" in
  --delete)
    need atlas "https://www.mongodb.com/docs/atlas/cli/current/install-atlas-cli/"
    atlas local delete "$NAME" --force
    exit 0 ;;
  --no-db) ;;
  "")
    need atlas "https://www.mongodb.com/docs/atlas/cli/current/install-atlas-cli/"
    if atlas local list 2>/dev/null | grep -q "$NAME"; then
      echo "== reusing the local deployment $NAME =="
      atlas local start "$NAME" >/dev/null 2>&1 || true
    else
      echo "== creating the local deployment $NAME on port $PORT (Atlas Search and Vector Search included) =="
      atlas local setup "$NAME" --port "$PORT" --force
    fi
    echo "The app connects to mongodb://localhost:${PORT}/?directConnection=true (delete it later: scripts/dev-desktop.sh --delete)." ;;
  *) echo "usage: scripts/dev-desktop.sh [--no-db | --delete]" >&2; exit 1 ;;
esac

echo "== installing web dependencies =="
(cd web && npm install --no-audit --no-fund)

echo "== opening the desktop app (the first Rust build takes a few minutes) =="
cd web && npm run tauri dev
