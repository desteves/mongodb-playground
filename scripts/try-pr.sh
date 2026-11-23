#!/usr/bin/env bash
# Try a pull request (for example a Dependabot update) on your machine before merging it. Every merge to main deploys.
#   scripts/try-pr.sh 5          check out PR #5 and run the checks that can fail on a dependency update
#   scripts/try-pr.sh 5 --full   also run the Atlas-backed tests and the browser tests (slower)
# It leaves you on the PR's branch; `git switch main` goes back. Needs the GitHub CLI (`gh`) signed in.
set -euo pipefail
cd "$(dirname "$0")/.."
PR=${1:?usage: scripts/try-pr.sh <pr-number> [--full]}
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

[ -z "$(git status --porcelain)" ] || { echo "commit or stash your changes first (git status is not clean)" >&2; exit 1; }
gh pr checkout "$PR"
echo "== on $(git branch --show-current): $(gh pr view "$PR" --json title -q .title) =="

if git diff --name-only main...HEAD | grep -qE '^(Cargo\.(toml|lock)|crates/)'; then
  echo "== Rust: format, lint, unit tests =="
  cargo fmt --all -- --check
  cargo clippy --workspace --exclude playground-desktop --all-targets -- -D warnings
  cargo test --workspace --exclude playground-desktop --lib --bins
fi
if git diff --name-only main...HEAD | grep -qE '^web/'; then
  echo "== Web: install, typecheck, unit tests, build =="
  (cd web && npm ci --no-audit --no-fund && npm run check && npm test && npm run build)
fi
if git diff --name-only main...HEAD | grep -qE '^Dockerfile'; then
  echo "== Docker image changed: building it =="
  docker build -f Dockerfile.api -t playground-try .
fi
if [ "${2:-}" = "--full" ]; then
  scripts/test-with-atlas-local.sh
  scripts/e2e.sh
fi
echo "== all checks passed for PR #$PR. Merge with: gh pr merge $PR --squash --delete-branch =="
