variable "project_id" { type = string }
variable "region" { type = string }
variable "service_account" { type = string }
variable "network" { type = string }
variable "subnetwork" { type = string }
variable "secret_ids" { type = map(string) }
variable "concurrency" { type = number }
variable "max_instances" { type = number }
variable "min_instances" { type = number }
variable "feature_sample_data" { type = bool }
variable "domain" { type = string }
variable "internal_token" {
  type      = string
  sensitive = true
}
variable "cluster_tier" {
  type        = string
  description = "Reported by the API (GET /api/v1/limits) and shown on the About page."
}

variable "turnstile_site_key" {
  type    = string
  default = ""
}
variable "image" {
  type        = string
  description = "Placeholder only: Cloud Build deploys the real image and Terraform ignores later changes."
  default     = "us-docker.pkg.dev/cloudrun/container/hello"
}

resource "google_cloud_run_v2_service" "api" {
  project  = var.project_id
  name     = "playground-api"
  location = var.region
  # Only the load balancer (and so Cloudflare, via the Armor rules) can reach it.
  ingress = "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER"

  template {
    service_account = var.service_account
    # Each run holds one MongoDB connection, so concurrency matches the connection pool.
    max_instance_request_concurrency = var.concurrency
    # Search-index runs can take up to 60 s; leave headroom.
    timeout = "120s"

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    # Direct VPC egress: the Atlas Private Service Connect endpoint lives in this subnet.
    vpc_access {
      egress = "PRIVATE_RANGES_ONLY"
      network_interfaces {
        network    = var.network
        subnetwork = var.subnetwork
      }
    }

    containers {
      image = var.image
      ports {
        container_port = 8080
      }
      resources {
        # More CPU while the container starts, so a scale-up request does not wait on a cold process.
        startup_cpu_boost = true
        # Results are uncapped, so leave memory headroom for big aggregations.
        limits = {
          cpu    = "1"
          memory = "2Gi"
        }
      }
      startup_probe {
        tcp_socket {
          port = 8080
        }
        period_seconds    = 2
        failure_threshold = 15
      }

      env {
        name  = "LOG_JSON"
        value = "1"
      }
      env {
        name  = "APP_ENV"
        value = "prod"
      }
      env {
        name  = "WEB_DIR"
        value = "/app/web"
      }
      env {
        name  = "PUBLIC_URL"
        value = "https://${var.domain}"
      }
      dynamic "env" {
        for_each = var.turnstile_site_key == "" ? [] : [var.turnstile_site_key]
        content {
          name  = "TURNSTILE_SITE_KEY"
          value = env.value
        }
      }
      dynamic "env" {
        for_each = var.feature_sample_data ? ["1"] : []
        content {
          name  = "FEATURE_SAMPLE_DATA"
          value = env.value
        }
      }
      env {
        name  = "CLUSTER_TIER"
        value = var.cluster_tier
      }

      dynamic "env" {
        for_each = var.secret_ids
        content {
          name = env.key
          value_source {
            secret_key_ref {
              secret  = env.value
              version = "latest"
            }
          }
        }
      }
    }
  }

  lifecycle {
    # Cloud Build owns the image (and its revision labels) after the first apply.
    ignore_changes = [
      template[0].containers[0].image,
      template[0].labels,
      template[0].annotations,
      client,
      client_version,
      # Terraform reads this service-level block back as 0 and then wants to remove it: noise, not a change.
      scaling,
    ]
  }
}

# The load balancer calls Cloud Run without credentials; the ingress setting above is the gate.
resource "google_cloud_run_v2_service_iam_member" "public" {
  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.api.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

# Scheduled jobs go through the public hostname (Cloudflare, then the load balancer) and
# authenticate with the shared internal token.
resource "google_cloud_scheduler_job" "sweep" {
  project     = var.project_id
  region      = var.region
  name        = "playground-sweep"
  schedule    = "*/10 * * * *"
  time_zone   = "UTC"
  description = "Drop pg_* databases left behind by a crash"
  http_target {
    http_method = "POST"
    uri         = "https://${var.domain}/internal/jobs/sweep"
    headers     = { "X-Internal-Token" = var.internal_token }
  }
}

resource "google_cloud_scheduler_job" "rollup" {
  project     = var.project_id
  region      = var.region
  name        = "playground-rollup"
  schedule    = "15 * * * *"
  time_zone   = "UTC"
  description = "Roll recent events up into daily_rollups"
  http_target {
    http_method = "POST"
    uri         = "https://${var.domain}/internal/jobs/rollup"
    headers     = { "X-Internal-Token" = var.internal_token }
  }
}

output "service_name" { value = google_cloud_run_v2_service.api.name }
output "service_url" { value = google_cloud_run_v2_service.api.uri }
