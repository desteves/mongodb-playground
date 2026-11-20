variable "project_id" {
  type        = string
  description = "GCP project ID (not the number)."
}

variable "region" {
  type    = string
  default = "us-central1"
}

variable "domain" {
  type        = string
  description = "Public hostname, served through Cloudflare."
  default     = "mongoplayground.com"
}

variable "cloudflare_zone_id" {
  type = string
}

variable "cloudflare_account_id" {
  type        = string
  description = "Cloudflare account ID, so Terraform creates the Turnstile widget (needs Account > Turnstile > Edit). Leave empty to reuse an existing widget via turnstile_site_key and turnstile_secret, or to skip Turnstile."
  default     = ""
}

variable "turnstile_site_key" {
  type        = string
  description = "Site key of an existing Turnstile widget (public). Empty uses the widget Terraform creates, if any."
  default     = ""
}

variable "turnstile_secret" {
  type        = string
  description = "Secret key of an existing Turnstile widget. Prefer TF_VAR_turnstile_secret over a file."
  sensitive   = true
  default     = ""
}

variable "atlas_project_id" {
  type        = string
  description = "Atlas project that holds the playground cluster. Keep this project dedicated: nothing else should live on the cluster."
}

variable "atlas_region" {
  type        = string
  description = "Atlas region name for the cluster (it must match var.region geographically)."
  default     = "CENTRAL_US"
}

variable "cluster_name" {
  type    = string
  default = "playground"
}

variable "cluster_tier" {
  type    = string
  default = "M20"
}

variable "termination_protection" {
  type    = bool
  default = true
}

variable "sample_databases" {
  type        = list(string)
  description = "Sample databases the read-only user may read. Load them with: atlas clusters sampleData load <cluster>."
  default     = ["sample_mflix", "sample_airbnb", "sample_restaurants", "sample_training", "sample_supplies", "sample_analytics", "sample_geospatial", "sample_guides", "sample_weatherdata"]
}

variable "max_pool_size" {
  type        = number
  description = "Mongo maxPoolSize per Cloud Run instance. Also used as Cloud Run concurrency: each run holds one connection."
  default     = 8
}

variable "max_instances" {
  type        = number
  description = "Cap on Cloud Run instances. Connections on the cluster = instances x (maxPoolSize + 2 monitors) x 3 members."
  default     = 20
}

variable "enforce_origin_restriction" {
  type        = bool
  description = "false = Cloud Armor origin rules only log (preview). Flip to true after confirming in the logs that real traffic matches; a wrong rule is an outage."
  default     = false
}

variable "enable_github_connection_setup" {
  type        = bool
  description = "Grant Cloud Build's service agent secretmanager.admin so `scripts/setup-cloudbuild-github.sh` can create the GitHub connection. Set false once the connection is COMPLETE to drop the grant."
  default     = true
}

variable "github_repository_id" {
  type        = string
  description = "Full name of the Cloud Build 2nd-gen linked repository: projects/<p>/locations/<r>/connections/<c>/repositories/<r>. Empty skips the trigger."
  default     = ""
}

variable "deploy_branch" {
  type    = string
  default = "main"
}

variable "notification_email" {
  type        = string
  description = "Where alerts go. Empty skips the alert policy."
  default     = ""
}

variable "billing_account" {
  type        = string
  description = "Billing account ID for the budget alert. Empty skips it."
  default     = ""
}

variable "budget_usd" {
  type    = number
  default = 300
}
