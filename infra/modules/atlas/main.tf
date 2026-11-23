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

  # Provider 2.x: nested blocks became attributes.
  replication_specs = [
    {
      region_configs = [
        {
          provider_name = "GCP"
          region_name   = var.atlas_region
          priority      = 7
          electable_specs = {
            instance_size = var.cluster_tier
            node_count    = 3
          }
          auto_scaling = {
            disk_gb_enabled = true
          }
        }
      ]
    }
  ]
}

# ---- Private Service Connect --------------------------------------------------------------
#
# Port-mapped architecture below (one address, one forwarding rule). The legacy architecture (one forwarding
# rule per Atlas node, 50 of them) was removed: Atlas disables it on 2027-04-30. NOTE: the project's legacy
# endpoint service in this region is SHARED with another application's endpoint group, so the Terraform
# resource that represented it (mongodbatlas_privatelink_endpoint.this) was taken out of the state with
# `terraform state rm`, never destroyed. Do not re-add it.

# ---- Port-mapped Private Service Connect (the supported architecture) ----------------------
#
# Atlas retires the legacy GCP architecture (one forwarding rule per node, the 50 resources we had before) on
# 2027-04-30. Port mapping needs one address and one forwarding rule in total: a single service attachment
# serves up to 150 nodes, one port per node. Connection hostnames are -psc-N (the legacy ones were -pl-N).
# Migration guide: https://registry.terraform.io/providers/mongodb/mongodbatlas/latest/docs/guides/gcp-privatelink-port-mapping-migration

resource "mongodbatlas_privatelink_endpoint" "port_mapped" {
  project_id           = var.atlas_project_id
  provider_name        = "GCP"
  region               = var.region
  port_mapping_enabled = true
}

resource "google_compute_address" "psc_port_mapped" {
  project      = var.project_id
  name         = "atlas-psc-port-mapped"
  region       = var.region
  subnetwork   = var.subnetwork_id
  address_type = "INTERNAL"
}

resource "google_compute_forwarding_rule" "psc_port_mapped" {
  project               = var.project_id
  name                  = google_compute_address.psc_port_mapped.name
  region                = var.region
  network               = var.network_id
  ip_address            = google_compute_address.psc_port_mapped.id
  target                = mongodbatlas_privatelink_endpoint.port_mapped.service_attachment_names[0]
  load_balancing_scheme = ""
}

resource "mongodbatlas_privatelink_endpoint_service" "port_mapped" {
  project_id                  = mongodbatlas_privatelink_endpoint.port_mapped.project_id
  private_link_id             = mongodbatlas_privatelink_endpoint.port_mapped.private_link_id
  provider_name               = "GCP"
  endpoint_service_id         = google_compute_forwarding_rule.psc_port_mapped.name
  private_endpoint_ip_address = google_compute_address.psc_port_mapped.address
  gcp_project_id              = var.project_id
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
  # Atlas publishes one private-endpoint SRV string per endpoint registered on the project's private
  # endpoint services (legacy -pl-N, port-mapped -psc-N). A project can hold other applications' endpoints
  # next to ours, and connecting through the wrong one times out, so pick the entry whose endpoint_id is OUR
  # port-mapped forwarding rule. If none matches the secret stays "pending" (see the
  # private_endpoint_candidates output) rather than silently using someone else's endpoint.
  private_endpoints = try(flatten([for cs in mongodbatlas_advanced_cluster.this.connection_strings.private_endpoint : cs]), [])
  mine = [
    for pe in local.private_endpoints : pe
    if pe.srv_connection_string != "" && contains([for e in pe.endpoints : e.endpoint_id], mongodbatlas_privatelink_endpoint_service.port_mapped.endpoint_service_id)
  ]
  private_srv = length(local.mine) > 0 ? local.mine[0].srv_connection_string : ""
  host        = local.private_srv == "" ? "" : regex("^mongodb\\+srv://([^/?]+)", local.private_srv)[0]
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

# Debugging aid: every private connection string Atlas offers and which endpoint IDs belong to it.
output "private_endpoint_candidates" {
  value = [
    for pe in local.private_endpoints : {
      srv          = pe.srv_connection_string
      endpoint_ids = [for e in pe.endpoints : e.endpoint_id]
    }
  ]
}

output "private_endpoint_ready" {
  value = local.host != ""
}
