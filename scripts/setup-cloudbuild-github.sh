#!/usr/bin/env bash
# One-time setup: connect Cloud Build to this GitHub repository and link it.
#
# Terraform cannot do the browser step (authorising the Cloud Build GitHub App), so this script does the
# gcloud part and prints the repository ID to put in infra/terraform.tfvars as github_repository_id.
#
# Order: terraform apply (grants the Cloud Build service agent secretmanager.admin)
#        -> scripts/setup-cloudbuild-github.sh
#        -> set github_repository_id in infra/terraform.tfvars -> terraform apply (creates the triggers).
#
# Usage: scripts/setup-cloudbuild-github.sh [owner/repo]    (default: desteves/mongodb-playground)
# Env:   GCP_PROJECT_ID (default: gcloud's current project), REGION (default us-central1),
#        CONNECTION (default github)
set -euo pipefail

REPO_SLUG="${1:-desteves/mongodb-playground}"
REGION="${REGION:-us-central1}"
CONNECTION="${CONNECTION:-github}"
PROJECT="${GCP_PROJECT_ID:-$(gcloud config get-value project 2>/dev/null)}"
REPO_NAME="${REPO_SLUG##*/}"

if [ -z "$PROJECT" ]; then
  echo "No project: set GCP_PROJECT_ID or run 'gcloud config set project <id>'." >&2
  exit 1
fi
echo "Project: $PROJECT  Region: $REGION  Connection: $CONNECTION  Repo: $REPO_SLUG"

gcloud services enable cloudbuild.googleapis.com secretmanager.googleapis.com --project="$PROJECT"

stage() {
  gcloud builds connections describe "$CONNECTION" --region="$REGION" --project="$PROJECT" \
    --format="value(installationState.stage)" 2>/dev/null || true
}

# 1. The connection. Creating it needs the Cloud Build service agent to have secretmanager.admin
#    (granted by terraform apply with enable_github_connection_setup = true).
if [ -z "$(stage)" ]; then
  echo "== creating the connection =="
  if ! gcloud builds connections create github "$CONNECTION" --region="$REGION" --project="$PROJECT"; then
    echo >&2
    echo "Creating the connection failed. If it mentions Secret Manager permissions, run 'terraform apply'" >&2
    echo "in infra/ first (it grants the service agent secretmanager.admin), wait a minute, and retry." >&2
    exit 1
  fi
fi

# 2. The browser step.
if [ "$(stage)" != "COMPLETE" ]; then
  ACTION_URI="$(gcloud builds connections describe "$CONNECTION" --region="$REGION" --project="$PROJECT" \
    --format="value(installationState.actionUri)")"
  echo
  echo "== authorise the Cloud Build GitHub App (one time, in a browser) =="
  echo "Open this URL, sign in to GitHub and install the app. Under 'Repository access' choose"
  echo "'Only select repositories' and add $REPO_NAME, or choose 'All repositories':"
  echo "  $ACTION_URI"
  echo
  echo "Waiting for the authorisation to complete (Ctrl-C to stop; run this script again afterwards)..."
  for _ in $(seq 1 90); do
    [ "$(stage)" = "COMPLETE" ] && break
    sleep 5
  done
  if [ "$(stage)" != "COMPLETE" ]; then
    echo "Not authorised yet. Finish the browser step, then run this script again." >&2
    exit 1
  fi
fi
echo "Connection is COMPLETE."

# 3. Link the repository.
if ! gcloud builds repositories describe "$REPO_NAME" --connection="$CONNECTION" --region="$REGION" \
  --project="$PROJECT" >/dev/null 2>&1; then
  echo "== linking $REPO_SLUG =="
  if ! gcloud builds repositories create "$REPO_NAME" --connection="$CONNECTION" --region="$REGION" \
    --project="$PROJECT" --remote-uri="https://github.com/${REPO_SLUG}.git"; then
    echo >&2
    echo "Linking failed. If it says the repository is not accessible to the Cloud Build GitHub App:" >&2
    echo "  1. Open https://github.com/settings/installations (or the organisation's installations page)" >&2
    echo "     and configure the Google Cloud Build app (the installation id is in the error above)." >&2
    echo "  2. Under 'Repository access' choose 'Only select repositories' and add ${REPO_NAME}," >&2
    echo "     or choose 'All repositories'. Save, then run this script again." >&2
    exit 1
  fi
fi

ID="projects/${PROJECT}/locations/${REGION}/connections/${CONNECTION}/repositories/${REPO_NAME}"
echo
echo "Done. Put this in infra/terraform.tfvars, then run terraform apply to create the triggers:"
echo "  github_repository_id = \"${ID}\""
echo
echo "Afterwards you may set enable_github_connection_setup = false to drop the service agent's"
echo "secretmanager.admin grant (the connection is already created)."
