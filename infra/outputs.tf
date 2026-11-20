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

output "site" {
  value = "https://${var.domain}"
}
