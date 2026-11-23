variable "project_id" { type = string }
variable "region" { type = string }
variable "domain" { type = string }
variable "cloud_run_service" { type = string }
variable "origin_key" {
  type      = string
  sensitive = true
}
variable "enforce_origin_restriction" { type = bool }
variable "enforce_origin_key" { type = bool }

# Cloudflare's published edge ranges (verified 2026-11-23 against
# https://www.cloudflare.com/ips-v4 and /ips-v6). Deliberately not fetched with a data source:
# that would make `plan` non-deterministic. RE-VERIFY before enabling enforcement: a range we
# have missed locks real visitors out.
locals {
  cloudflare_ipv4 = [
    "173.245.48.0/20", "103.21.244.0/22", "103.22.200.0/22", "103.31.4.0/22", "141.101.64.0/18",
    "108.162.192.0/18", "190.93.240.0/20", "188.114.96.0/20", "197.234.240.0/22", "198.41.128.0/17",
    "162.158.0.0/15", "104.16.0.0/13", "104.24.0.0/14", "172.64.0.0/13", "131.0.72.0/22",
  ]
  cloudflare_ipv6 = [
    "2400:cb00::/32", "2606:4700::/32", "2803:f800::/32", "2405:b500::/32", "2405:8100::/32",
    "2a06:98c0::/29", "2c0f:f248::/32",
  ]
  # A basic src_ip_ranges match takes at most 10 entries.
  ipv4_a = slice(local.cloudflare_ipv4, 0, 10)
  ipv4_b = slice(local.cloudflare_ipv4, 10, length(local.cloudflare_ipv4))

  # Query bodies are JSON like {"$where": ...} or {"name": /<script>/}: the OWASP XSS/SQLi/LFI
  # rules read them as attacks (a common false positive for JSON APIs). So the
  # API surface is excluded from those rules and protected by the app's allowlist, Cloudflare
  # rate limiting, and the origin lock below. Everything else (the SPA, unknown paths) keeps them.
  not_api = "!request.path.startsWith('/api/') && request.path != '/mcp' && request.path != '/internal/jobs/sweep' && request.path != '/internal/jobs/rollup'"
}

resource "google_compute_global_address" "lb" {
  project = var.project_id
  name    = "playground-lb-ip"
}

resource "google_compute_ssl_policy" "modern" {
  project         = var.project_id
  name            = "playground-modern-tls"
  profile         = "RESTRICTED"
  min_tls_version = "TLS_1_2"
}

# Covers the apex and www: Cloudflare is set to Full (strict), so every proxied hostname needs a valid
# origin certificate. A managed certificate cannot be edited in place, so changing the domain list makes a
# new one. The random suffix changes with the domains, and create_before_destroy lets the new certificate
# exist while the old one is still attached to the load balancer.
locals {
  cert_domains = [var.domain, "www.${var.domain}"]
}

resource "random_id" "cert" {
  byte_length = 3
  keepers = {
    domains = join(",", local.cert_domains)
  }
}

resource "google_compute_managed_ssl_certificate" "cert" {
  project = var.project_id
  name    = "playground-cert-${random_id.cert.hex}"
  managed {
    domains = local.cert_domains
  }
  lifecycle {
    create_before_destroy = true
  }
}

resource "google_compute_security_policy" "armor" {
  project     = var.project_id
  name        = "playground-armor"
  description = "WAF and origin lock for MongoDB Playground"

  adaptive_protection_config {
    layer_7_ddos_defense_config {
      enable = true
    }
  }

  rule {
    action   = "deny(403)"
    priority = 2000
    match {
      expr {
        expression = "${local.not_api} && evaluatePreconfiguredExpr('xss-v33-stable')"
      }
    }
    description = "OWASP XSS (not on API paths: query bodies look like attacks)"
  }

  rule {
    action   = "deny(403)"
    priority = 2010
    match {
      expr {
        expression = "${local.not_api} && evaluatePreconfiguredExpr('sqli-v33-stable')"
      }
    }
    description = "OWASP SQLi (not on API paths)"
  }

  rule {
    action   = "deny(403)"
    priority = 2020
    match {
      expr {
        expression = "${local.not_api} && (evaluatePreconfiguredExpr('lfi-v33-stable') || evaluatePreconfiguredExpr('rfi-v33-stable'))"
      }
    }
    description = "OWASP LFI/RFI (not on API paths)"
  }

  rule {
    action   = "deny(403)"
    priority = 2030
    match {
      expr {
        expression = "evaluatePreconfiguredExpr('scannerdetection-v33-stable')"
      }
    }
    description = "Vulnerability scanner detection"
  }

  # Origin lock, two independent checks: the header
  # proves the request came through OUR Cloudflare zone (any Cloudflare customer can point at our
  # IP and pass the range check); the ranges are defence in depth. They sit at 2100+, AFTER the
  # denies, because an allow ends evaluation.
  rule {
    action   = "deny(403)"
    priority = 2100
    # Stage 1 (enforce_origin_key) turns this rule on by itself; stage 2 (enforce_origin_restriction) also denies
    # everything that is not from a Cloudflare range. Stage 2 includes stage 1.
    preview = !(var.enforce_origin_key || var.enforce_origin_restriction)
    match {
      expr {
        expression = "!has(request.headers['x-origin-key']) || request.headers['x-origin-key'] != '${var.origin_key}'"
      }
    }
    description = "Deny requests that did not come through our Cloudflare zone"
  }

  rule {
    action   = "allow"
    priority = 2110
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = local.ipv4_a
      }
    }
    description = "Cloudflare IPv4 (1/2)"
  }

  rule {
    action   = "allow"
    priority = 2111
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = local.ipv4_b
      }
    }
    description = "Cloudflare IPv4 (2/2)"
  }

  rule {
    action   = "allow"
    priority = 2112
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = local.cloudflare_ipv6
      }
    }
    description = "Cloudflare IPv6"
  }

  # Default: allow while the origin rules are in preview, deny once enforced.
  rule {
    action   = var.enforce_origin_restriction ? "deny(403)" : "allow"
    priority = 2147483647
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = ["*"]
      }
    }
    description = "Default rule"
  }
}

resource "google_compute_region_network_endpoint_group" "api" {
  project               = var.project_id
  name                  = "playground-api-neg"
  region                = var.region
  network_endpoint_type = "SERVERLESS"
  cloud_run {
    service = var.cloud_run_service
  }
}

resource "google_compute_backend_service" "api" {
  project               = var.project_id
  name                  = "playground-api"
  protocol              = "HTTPS"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  security_policy       = google_compute_security_policy.armor.id
  # No timeout_sec: serverless NEG backends reject it. The request limit is Cloud Run's own timeout
  # (search-index runs can take 60 s, so keep that at 120 s or more).
  backend {
    group = google_compute_region_network_endpoint_group.api.id
  }
  log_config {
    enable      = true
    sample_rate = 1.0
  }
}

# The SPA is served by the API container too (static files with an index.html fallback), so a
# single backend covers everything; Cloudflare caches the hashed assets.
resource "google_compute_url_map" "map" {
  project         = var.project_id
  name            = "playground"
  default_service = google_compute_backend_service.api.id
}

resource "google_compute_target_https_proxy" "https" {
  project          = var.project_id
  name             = "playground-https"
  url_map          = google_compute_url_map.map.id
  ssl_certificates = [google_compute_managed_ssl_certificate.cert.id]
  ssl_policy       = google_compute_ssl_policy.modern.id
}

resource "google_compute_global_forwarding_rule" "https" {
  project               = var.project_id
  name                  = "playground-https"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_address            = google_compute_global_address.lb.id
  port_range            = "443"
  target                = google_compute_target_https_proxy.https.id
}

output "load_balancer_ip" {
  value = google_compute_global_address.lb.address
}
