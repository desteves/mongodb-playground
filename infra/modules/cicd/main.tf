variable "project_id" { type = string }
variable "region" { type = string }
variable "github_repository_id" { type = string }
variable "deploy_branch" { type = string }
variable "repository_url" { type = string }
variable "enable_github_connection_setup" { type = bool }

locals {
  enabled = var.github_repository_id != ""
}

data "google_project" "this" {
  project_id = var.project_id
}

# ---- Before the GitHub connection can be created -----------------------------------------------------
# `gcloud builds connections create github` has Cloud Build's own service agent store the GitHub token in
# Secret Manager, so the agent needs secretmanager.admin (it needs secrets.create and secrets.setIamPolicy).
# The connection itself needs a human to authorise the GitHub App in a browser, so it is created by
# scripts/setup-cloudbuild-github.sh, not here. Order: apply (this grant) -> run the script -> set
# github_repository_id -> apply again (creates the triggers). After the connection is COMPLETE you can set
# enable_github_connection_setup = false to drop the grant.

# The service agent only exists once something has asked for it; this makes sure it does.
resource "terraform_data" "cloudbuild_agent" {
  count            = var.enable_github_connection_setup ? 1 : 0
  triggers_replace = [var.project_id]

  provisioner "local-exec" {
    command = "gcloud beta services identity create --service=cloudbuild.googleapis.com --project=${var.project_id}"
  }
}

resource "google_project_iam_member" "cloudbuild_agent_secrets" {
  count   = var.enable_github_connection_setup ? 1 : 0
  project = var.project_id
  role    = "roles/secretmanager.admin"
  member  = "serviceAccount:service-${data.google_project.this.number}@gcp-sa-cloudbuild.iam.gserviceaccount.com"

  depends_on = [terraform_data.cloudbuild_agent]
}

# 2nd-gen Cloud Build triggers need a user-managed service account (Google-managed ones are
# rejected), so the pipeline runs as a dedicated, narrowly scoped one.
resource "google_service_account" "build" {
  count        = local.enabled ? 1 : 0
  project      = var.project_id
  account_id   = "playground-deployer"
  display_name = "Cloud Build deploys (push to ${var.deploy_branch})"
}

resource "google_project_iam_member" "build" {
  for_each = local.enabled ? toset([
    "roles/artifactregistry.writer",
    "roles/logging.logWriter",
    "roles/run.admin",
    "roles/iam.serviceAccountUser",
  ]) : toset([])
  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.build[0].email}"
}

# On push to the deploy branch: test, build, push, deploy (cloudbuild.yaml at the repo root).
resource "google_cloudbuild_trigger" "deploy" {
  count           = local.enabled ? 1 : 0
  project         = var.project_id
  location        = var.region
  name            = "deploy-playground-${var.deploy_branch}"
  description     = "Test, build and deploy MongoDB Playground on push to ${var.deploy_branch}"
  filename        = "cloudbuild.yaml"
  service_account = "projects/${var.project_id}/serviceAccounts/${google_service_account.build[0].email}"

  ignored_files = ["docs/**", "infra/**", "*.md", ".claude/**"]

  substitutions = {
    _REGION  = var.region
    _IMAGE   = "${var.repository_url}/api"
    _SERVICE = "playground-api"
  }

  repository_event_config {
    repository = var.github_repository_id
    push {
      branch = "^${var.deploy_branch}$"
    }
  }

  depends_on = [google_project_iam_member.build]
}
