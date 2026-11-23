output "load_balancer_ip" {
  value = module.edge.load_balancer_ip
}

output "service_url" {
  value = module.runtime.service_url
}

output "artifact_repository_url" {
  value = module.bootstrap.repository_url
}

output "atlas_private_endpoint_ready" {
  description = "false until Atlas reports the private endpoint connection string (PSC takes a few minutes)."
  value       = module.atlas.private_endpoint_ready
}

output "atlas_private_endpoint_candidates" {
  description = "Every private connection string Atlas offers for the cluster and the endpoint IDs behind each. Ours is the one whose endpoint_ids contain the VPC name (playground)."
  value       = module.atlas.private_endpoint_candidates
}

output "site" {
  value = "https://${var.domain}"
}
