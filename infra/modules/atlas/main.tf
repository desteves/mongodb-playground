terraform {
  required_providers {
    mongodbatlas = { source = "mongodb/mongodbatlas" }
    random       = { source = "hashicorp/random" }
    google       = { source = "hashicorp/google" }
  }
}

variable "project_id" { type = string }
variable "region" { type = string }
variable "atlas_project_id" { type = string }
variable "atlas_region" { type = string }
variable "cluster_name" { type = string }
variable "cluster_tier" { type = string }
variable "network_id" { type = string }
variable "network_name" { type = string }
variable "subnetwork_id" { type = string }
variable "sample_databases" { type = list(string) }
variable "max_pool_size" { type = number }
variable "termination_protection" { type = bool }

resource "mongodbatlas_advanced_cluster" "this" {
  project_id                     = var.atlas_project_id
  name                           = var.cluster_name
  cluster_type                   = "REPLICASET"
  backup_enabled                 = true
  termination_protection_enabled = var.termination_protection

  replication_specs {
    region_configs {
      provider_name = "GCP"
      region_name   = var.atlas_region
      priority      = 7
      electable_specs {
        instance_size = var.cluster_tier
        node_count    = 3
      }
      auto_scaling {
        disk_gb_enabled = true
      }
    }
  }
}

# ---- Private Service Connect --------------------------------------------------------------
#
# Legacy architecture: one internal address and one forwarding rule per Atlas service
# attachment (50). The provider version pinned here (1.x) predates `port_mapping_enabled`, which
# would need only one of each; moving to the 2.x provider is the upgrade path (it also changes the
# mongodbatlas_advanced_cluster schema, so do both together).

resource "mongodbatlas_privatelink_endpoint" "this" {
  project_id    = var.atlas_project_id
  provider_name = "GCP"
  region        = var.region
}

resource "google_compute_address" "psc" {
  count        = 50
  project      = var.project_id
  name         = "atlas-psc-${count.index}"
  region       = var.region
  subnetwork   = var.subnetwork_id
  address_type = "INTERNAL"
  depends_on   = [mongodbatlas_privatelink_endpoint.this]
}

resource "google_compute_forwarding_rule" "psc" {
  count                 = 50
  project               = var.project_id
  name                  = google_compute_address.psc[count.index].name
  region                = var.region
  network               = var.network_id
  ip_address            = google_compute_address.psc[count.index].id
  target                = mongodbatlas_privatelink_endpoint.this.service_attachment_names[count.index]
  load_balancing_scheme = ""
}

resource "mongodbatlas_privatelink_endpoint_service" "this" {
  project_id          = var.atlas_project_id
  private_link_id     = mongodbatlas_privatelink_endpoint.this.private_link_id
  provider_name       = "GCP"
  endpoint_service_id = var.network_name
  gcp_project_id      = var.project_id

  dynamic "endpoints" {
    for_each = google_compute_address.psc
    content {
      ip_address    = endpoints.value.address
      endpoint_name = google_compute_forwarding_rule.psc[endpoints.key].name
    }
  }

  depends_on = [google_compute_forwarding_rule.psc]
}

# ---- One database user per job, each with only what it needs ---------------------------------
#
# runner: creates and drops the throwaway `pg_*` databases, builds indexes and search indexes.
# Atlas roles cannot be scoped to a database-name prefix, so the Rust validator is what pins a
# run to its own database; this user must therefore only ever exist on this dedicated cluster.
# VERIFY on the first deploy that these built-in roles grant dropDatabase, createIndex and the
# search-index actions; if not, replace them with a mongodbatlas_custom_db_role.

locals {
  users = {
    runner = {
      roles = [
        { role = "readWriteAnyDatabase", db = "admin" },
        { role = "dbAdminAnyDatabase", db = "admin" },
      ]
    }
    sample = {
      roles = [for d in var.sample_databases : { role = "read", db = d }]
    }
    shares = {
      roles = [{ role = "readWrite", db = "playground_shares" }]
    }
    stats = {
      roles = [{ role = "readWrite", db = "playground-platform" }]
    }
  }
}

resource "random_password" "user" {
  for_each = local.users
  length   = 32
  special  = false
}

resource "mongodbatlas_database_user" "user" {
  for_each           = local.users
  project_id         = var.atlas_project_id
  username           = "pg-${each.key}"
  password           = random_password.user[each.key].result
  auth_database_name = "admin"

  dynamic "roles" {
    for_each = each.value.roles
    content {
      role_name     = roles.value.role
      database_name = roles.value.db
    }
  }

  scopes {
    name = mongodbatlas_advanced_cluster.this.name
    type = "CLUSTER"
  }
}

locals {
  # Atlas publishes the private-endpoint-aware SRV string once the PSC connection is available.
  private_srv = try(
    [for pe in mongodbatlas_advanced_cluster.this.connection_strings[0].private_endpoint : pe.srv_connection_string if pe.srv_connection_string != ""][0],
    "",
  )
  host = local.private_srv == "" ? "" : regex("^mongodb\\+srv://([^/?]+)", local.private_srv)[0]
  # Connection-string best practice: compression, retry, small pool and an app name are set here
  # so operators can retune them without a rebuild (the app only fills in what is missing).
  params = "retryWrites=true&w=majority&appName=mongodb-playground&compressors=zstd,snappy,zlib&maxPoolSize=${var.max_pool_size}&maxIdleTimeMS=30000"
}

output "uris" {
  description = "Connection strings by user, over the private endpoint. Empty until the PSC connection is ready: apply again."
  sensitive   = true
  value = {
    for name, _ in local.users :
    name => local.host == "" ? "" : "mongodb+srv://pg-${name}:${random_password.user[name].result}@${local.host}/?${local.params}"
  }
}

output "private_endpoint_ready" {
  value = local.host != ""
}
