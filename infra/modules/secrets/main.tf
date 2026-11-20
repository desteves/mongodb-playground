variable "project_id" { type = string }
variable "runtime_sa" { type = string }
variable "atlas_uris" {
  type      = map(string)
  sensitive = true
}
variable "internal_token" {
  type      = string
  sensitive = true
}
variable "enable_turnstile" {
  type    = bool
  default = false
}
variable "turnstile_secret" {
  type      = string
  sensitive = true
  default   = ""
}
variable "session_secret" {
  type      = string
  sensitive = true
}
variable "stats_salt" {
  type      = string
  sensitive = true
}

locals {
  # secret id -> value. An Atlas URI is empty until the PSC connection is ready; "pending" keeps
  # the secret (and so the Cloud Run revision) valid, and the next apply replaces it.
  names = {
    "mongodb-uri"        = "runner"
    "mongodb-sample-uri" = "sample"
    "mongodb-shares-uri" = "shares"
    "mongodb-stats-uri"  = "stats"
  }
}

resource "google_secret_manager_secret" "uri" {
  for_each  = local.names
  project   = var.project_id
  secret_id = each.key
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "uri" {
  for_each    = local.names
  secret      = google_secret_manager_secret.uri[each.key].id
  secret_data = var.atlas_uris[each.value] == "" ? "pending" : var.atlas_uris[each.value]
}

resource "google_secret_manager_secret" "plain" {
  for_each  = toset(["internal-token", "stats-salt"])
  project   = var.project_id
  secret_id = each.key
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "session" {
  project   = var.project_id
  secret_id = "session-secret"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "session" {
  secret      = google_secret_manager_secret.session.id
  secret_data = var.session_secret
}

resource "google_secret_manager_secret" "turnstile" {
  count     = var.enable_turnstile ? 1 : 0
  project   = var.project_id
  secret_id = "turnstile-secret"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "turnstile" {
  count       = var.enable_turnstile ? 1 : 0
  secret      = google_secret_manager_secret.turnstile[0].id
  secret_data = var.turnstile_secret
}

resource "google_secret_manager_secret_iam_member" "runtime_extra" {
  for_each = merge(
    { session = google_secret_manager_secret.session.id },
    var.enable_turnstile ? { turnstile = google_secret_manager_secret.turnstile[0].id } : {},
  )
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${var.runtime_sa}"
}

resource "google_secret_manager_secret_version" "internal_token" {
  secret      = google_secret_manager_secret.plain["internal-token"].id
  secret_data = var.internal_token
}

resource "google_secret_manager_secret_version" "stats_salt" {
  secret      = google_secret_manager_secret.plain["stats-salt"].id
  secret_data = var.stats_salt
}

resource "google_secret_manager_secret_iam_member" "runtime" {
  for_each = merge(
    { for k, v in google_secret_manager_secret.uri : k => v.id },
    { for k, v in google_secret_manager_secret.plain : k => v.id },
  )
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${var.runtime_sa}"
}

# environment variable name -> secret id, for the runtime module
output "secret_ids" {
  value = merge(
    {
      MONGODB_URI        = google_secret_manager_secret.uri["mongodb-uri"].secret_id
      MONGODB_SAMPLE_URI = google_secret_manager_secret.uri["mongodb-sample-uri"].secret_id
      MONGODB_SHARES_URI = google_secret_manager_secret.uri["mongodb-shares-uri"].secret_id
      STATS_MONGODB_URI  = google_secret_manager_secret.uri["mongodb-stats-uri"].secret_id
      INTERNAL_TOKEN     = google_secret_manager_secret.plain["internal-token"].secret_id
      STATS_SALT         = google_secret_manager_secret.plain["stats-salt"].secret_id
      SESSION_SECRET     = google_secret_manager_secret.session.secret_id
    },
    var.enable_turnstile ? { TURNSTILE_SECRET = google_secret_manager_secret.turnstile[0].secret_id } : {},
  )
}
