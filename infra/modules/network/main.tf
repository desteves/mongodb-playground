variable "project_id" { type = string }
variable "region" { type = string }

# One VPC. The Cloud Run service reaches Atlas through a Private Service Connect endpoint in
# this subnet (Direct VPC egress), so no connector, NAT or IP allow-list is needed.
resource "google_compute_network" "vpc" {
  project                 = var.project_id
  name                    = "playground"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "main" {
  project = var.project_id
  name    = "playground-${var.region}"
  region  = var.region
  network = google_compute_network.vpc.id
  # Room for 50 Private Service Connect addresses plus Cloud Run Direct VPC egress instances.
  ip_cidr_range            = "10.20.0.0/22"
  private_ip_google_access = true
}

output "network_id" { value = google_compute_network.vpc.id }
output "network_name" { value = google_compute_network.vpc.name }
output "subnetwork_id" { value = google_compute_subnetwork.main.id }
output "subnetwork_name" { value = google_compute_subnetwork.main.name }
