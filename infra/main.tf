terraform {
  required_version = ">= 1.5.0"

  # Partial configuration: terraform init -backend-config="bucket=<state bucket>" (see infra/README.md)
  backend "gcs" {
    prefix = "mongodb-playground"
  }

  required_providers {
    google       = { source = "hashicorp/google", version = "~> 6.0" }
    mongodbatlas = { source = "mongodb/mongodbatlas", version = ">= 2.22, < 3.0" }
    cloudflare   = { source = "cloudflare/cloudflare", version = "~> 4.30" }
    random       = { source = "hashicorp/random", version = "~> 3.6" }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

# Credentials come from the environment, never from files in the repo:
#   MONGODB_ATLAS_PUBLIC_API_KEY / MONGODB_ATLAS_PRIVATE_API_KEY, CLOUDFLARE_API_TOKEN
provider "mongodbatlas" {}
provider "cloudflare" {}

# Shared secrets generated once and kept in state (state bucket must be private and encrypted).
resource "random_password" "origin_key" {
  length  = 40
  special = false
}

resource "random_password" "internal_token" {
  length  = 40
  special = false
}

resource "random_password" "session_secret" {
  length  = 48
  special = false
}

resource "random_password" "stats_salt" {
  length  = 32
  special = false
}

module "bootstrap" {
  source     = "./modules/bootstrap"
  project_id = var.project_id
  region     = var.region
}

module "network" {
  source     = "./modules/network"
  project_id = var.project_id
  region     = var.region
  depends_on = [module.bootstrap]
}

# Atlas cluster, PrivateLink (Private Service Connect) into our VPC, and one DB user per job.
module "atlas" {
  source                 = "./modules/atlas"
  project_id             = var.project_id
  region                 = var.region
  atlas_project_id       = var.atlas_project_id
  atlas_region           = var.atlas_region
  cluster_name           = var.cluster_name
  cluster_tier           = var.cluster_tier
  network_id             = module.network.network_id
  network_name           = module.network.network_name
  subnetwork_id          = module.network.subnetwork_id
  sample_databases       = var.sample_databases
  max_pool_size          = var.max_pool_size
  termination_protection = var.termination_protection
  depends_on             = [module.network]
}

module "secrets" {
  source         = "./modules/secrets"
  project_id     = var.project_id
  runtime_sa     = module.iam.runtime_email
  atlas_uris     = module.atlas.uris
  internal_token = random_password.internal_token.result
  stats_salt     = random_password.stats_salt.result
  session_secret = random_password.session_secret.result
  # Turnstile is optional: without it everyone gets the verified rate-limit tier. Either reuse an
  # existing widget (turnstile_site_key + turnstile_secret) or let Terraform create one
  # (cloudflare_account_id, which needs the Account > Turnstile > Edit token permission).
  # Known at plan time (count cannot depend on a value that only exists after apply).
  # nonsensitive(): only "is it set?" is exposed, never the secret, and for_each needs a non-sensitive value.
  enable_turnstile = var.cloudflare_account_id != "" || nonsensitive(var.turnstile_secret != "")
  turnstile_secret = local.turnstile_secret
  depends_on       = [module.bootstrap]
}

module "iam" {
  source     = "./modules/iam"
  project_id = var.project_id
  depends_on = [module.bootstrap]
}

module "runtime" {
  source          = "./modules/runtime"
  project_id      = var.project_id
  region          = var.region
  service_account = module.iam.runtime_email
  network         = module.network.network_name
  subnetwork      = module.network.subnetwork_name
  secret_ids      = module.secrets.secret_ids
  concurrency     = var.max_pool_size
  cluster_tier    = var.cluster_tier
  max_instances   = var.max_instances
  domain          = var.domain
  internal_token  = random_password.internal_token.result
  # Empty when Turnstile is off.
  turnstile_site_key = local.turnstile_site_key
  depends_on         = [module.secrets, module.network]
}

module "edge" {
  source                     = "./modules/edge"
  project_id                 = var.project_id
  region                     = var.region
  domain                     = var.domain
  cloud_run_service          = module.runtime.service_name
  origin_key                 = random_password.origin_key.result
  enforce_origin_restriction = var.enforce_origin_restriction
  depends_on                 = [module.runtime]
}

module "cloudflare" {
  source           = "./modules/cloudflare"
  zone_id          = var.cloudflare_zone_id
  account_id       = var.cloudflare_account_id
  domain           = var.domain
  load_balancer_ip = module.edge.load_balancer_ip
  proxied          = var.cloudflare_proxied

  remove_conflicting_dns = var.cloudflare_remove_conflicting_dns
  origin_key             = random_password.origin_key.result
}

module "cicd" {
  source               = "./modules/cicd"
  project_id           = var.project_id
  region               = var.region
  github_repository_id = var.github_repository_id
  deploy_branch        = var.deploy_branch
  repository_url       = module.bootstrap.repository_url

  enable_github_connection_setup = var.enable_github_connection_setup
  depends_on                     = [module.bootstrap]
}

module "observability" {
  source             = "./modules/observability"
  project_id         = var.project_id
  domain             = var.domain
  notification_email = var.notification_email
  billing_account    = var.billing_account
  budget_usd         = var.budget_usd
}

locals {
  turnstile_site_key = var.turnstile_site_key != "" ? var.turnstile_site_key : module.cloudflare.turnstile_site_key
  turnstile_secret   = var.turnstile_secret != "" ? var.turnstile_secret : module.cloudflare.turnstile_secret
}
