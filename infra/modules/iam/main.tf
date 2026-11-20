variable "project_id" { type = string }

# The identity the API runs as. It can read its own secrets (granted in the secrets module) and
# write logs and metrics; nothing else.
resource "google_service_account" "runtime" {
  project      = var.project_id
  account_id   = "playground-api"
  display_name = "MongoDB Playground API (Cloud Run)"
}

resource "google_project_iam_member" "runtime_roles" {
  for_each = toset(["roles/logging.logWriter", "roles/monitoring.metricWriter"])
  project  = var.project_id
  role     = each.value
  member   = "serviceAccount:${google_service_account.runtime.email}"
}

output "runtime_email" {
  value = google_service_account.runtime.email
}
