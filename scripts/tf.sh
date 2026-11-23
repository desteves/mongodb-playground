#!/usr/bin/env bash
# Terraform wrapper that keeps saved plans OUT of the repository.
#
# A saved plan file contains the whole prior state, so one committed to a public repo leaks every secret
# Terraform manages. This script writes the plan to a private directory outside the checkout, applies exactly
# that plan, and deletes it afterwards. It also loads ../.env (API keys) so every shell behaves the same.
#
#   scripts/tf.sh plan [terraform plan args]    # e.g. -replace=... ; saves the plan outside the repo; uses ATLAS_READONLY_* if set
#   scripts/tf.sh apply                          # applies the plan saved by the last `plan`, then removes it
#   scripts/tf.sh <anything else>                # passed through to terraform in infra/ (init, output, state, ...)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLAN_DIR="${TF_PLAN_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/mongodb-playground/tfplans}"
case "$PLAN_DIR/" in
  "$ROOT"/*) echo "tf.sh: TF_PLAN_DIR must be outside the repository ($ROOT)" >&2; exit 1 ;;
esac
mkdir -p "$PLAN_DIR" && chmod 700 "$PLAN_DIR"
PLAN="$PLAN_DIR/latest.tfplan"

if [ -f "$ROOT/.env" ]; then set -a; . "$ROOT/.env"; set +a; fi
tf() { terraform -chdir="$ROOT/infra" "$@"; }

cmd="${1:-}"
case "$cmd" in
  plan)
    shift
    # Planning only reads, so use the read-only Atlas key when one is configured (ATLAS_READONLY_* in .env).
    if [ -n "${ATLAS_READONLY_PUBLIC_API_KEY:-}" ] && [ -n "${ATLAS_READONLY_PRIVATE_API_KEY:-}" ]; then
      export MONGODB_ATLAS_PUBLIC_API_KEY="$ATLAS_READONLY_PUBLIC_API_KEY"
      export MONGODB_ATLAS_PRIVATE_API_KEY="$ATLAS_READONLY_PRIVATE_API_KEY"
      echo "tf.sh: planning with the read-only Atlas key"
    fi
    tf plan -out="$PLAN" "$@"
    chmod 600 "$PLAN"
    echo "plan saved outside the repo; apply it with: scripts/tf.sh apply"
    ;;
  apply)
    [ -f "$PLAN" ] || { echo "tf.sh: no saved plan; run scripts/tf.sh plan first" >&2; exit 1; }
    tf apply "$PLAN"
    rm -f "$PLAN"
    ;;
  "")
    echo "usage: scripts/tf.sh plan|apply|<terraform command>" >&2; exit 1 ;;
  *)
    tf "$@" ;;
esac
