terraform {
  required_providers {
    cloudflare = { source = "cloudflare/cloudflare" }
  }
}

variable "zone_id" { type = string }
variable "account_id" {
  type        = string
  description = "Cloudflare account ID. Empty skips the Turnstile widget (the app then gives everyone the verified rate-limit tier)."
  default     = ""
}
variable "domain" { type = string }
variable "load_balancer_ip" { type = string }
variable "remove_conflicting_dns" {
  type        = bool
  description = "Delete any A, AAAA or CNAME on the apex and www that is not an A record for the load balancer (for example a parked page left from a previous host). Needs CLOUDFLARE_API_TOKEN and python3 on the machine running Terraform."
  default     = true
}
variable "proxied" {
  type        = bool
  description = "Proxy the apex and www through Cloudflare. Turn off until the Google-managed certificate is ACTIVE: Google cannot validate a domain that resolves to Cloudflare."
  default     = true
}
variable "origin_key" {
  type      = string
  sensitive = true
}

# The apex and www must resolve to the load balancer and nothing else: a second A record (an old host's
# parking page) sends some visitors elsewhere, and Google cannot issue the load balancer's certificate
# while the name points somewhere else. This removes such records before ours are managed. It runs on the
# first apply and again whenever the zone, domain or load balancer IP changes. TXT records (SPF, DKIM,
# DMARC), MX records and other names are never touched.
resource "terraform_data" "dns_cleanup" {
  count            = var.remove_conflicting_dns ? 1 : 0
  triggers_replace = [var.zone_id, var.domain, var.load_balancer_ip]

  provisioner "local-exec" {
    command = "python3 ${path.module}/cleanup-dns.py ${var.zone_id} ${var.domain} ${var.load_balancer_ip}"
  }
}

resource "cloudflare_record" "apex" {
  depends_on = [terraform_data.dns_cleanup]
  zone_id    = var.zone_id
  name       = "@"
  type       = "A"
  content    = var.load_balancer_ip
  proxied    = var.proxied
  ttl        = 1
}

resource "cloudflare_record" "www" {
  depends_on = [terraform_data.dns_cleanup]
  zone_id    = var.zone_id
  name       = "www"
  type       = "A"
  content    = var.load_balancer_ip
  proxied    = var.proxied
  ttl        = 1
}

resource "cloudflare_zone_settings_override" "tls" {
  zone_id = var.zone_id
  settings {
    ssl              = "strict"
    always_use_https = "on"
    min_tls_version  = "1.2"
    tls_1_3          = "on"
    http3            = "on"
    brotli           = "on"
  }
}

# Sets X-Origin-Key on every request that reaches the origin. "set" overwrites, so a client
# cannot supply its own, and the Cloud Armor rule can trust it.
resource "cloudflare_ruleset" "origin_key" {
  zone_id     = var.zone_id
  name        = "Origin key"
  description = "Prove to the origin that the request came through this zone"
  kind        = "zone"
  phase       = "http_request_late_transform"

  rules {
    action      = "rewrite"
    expression  = "true"
    description = "Set X-Origin-Key"
    enabled     = true
    action_parameters {
      headers {
        name      = "X-Origin-Key"
        operation = "set"
        value     = var.origin_key
      }
    }
  }
}

# Rate limit the endpoints that do real work. (Free plan: 10 s window, colo+IP characteristics.)
resource "cloudflare_ruleset" "rate_limit" {
  zone_id     = var.zone_id
  name        = "Playground rate limits"
  description = "Throttle run, explain, share and MCP"
  kind        = "zone"
  phase       = "http_ratelimit"

  rules {
    action      = "block"
    expression  = "(http.request.method eq \"POST\" and (starts_with(http.request.uri.path, \"/api/v1/\") or http.request.uri.path eq \"/mcp\"))"
    description = "30 requests per 10 s per client"
    enabled     = true
    ratelimit {
      characteristics     = ["cf.colo.id", "ip.src"]
      period              = 10
      requests_per_period = 30
      mitigation_timeout  = 10
    }
  }
}

# Cache rules, ordered: when several rules match, a LATER rule wins for the same setting, so the bypass
# comes first and the specific allow-lists after it.
#
# What is cached, and why (see "Caching" in the README):
#   - hashed build output and static files: yes (names change on deploy, or the TTL is short);
#   - Markdown twins (/about.md, ...): yes, one URL means one representation;
#   - discovery documents and /api/v1/limits: yes, following the origin's short Cache-Control;
#   - shared playgrounds: yes, but at most a day (they expire 90 days after their last origin view);
#   - HTML pages: NEVER. The same URL answers HTML or Markdown depending on the Accept header, and a cache
#     key that ignores Accept would hand the wrong one to the next visitor (header cache keys need Enterprise);
#   - everything that does work (POSTs, /mcp, health, jobs): never.
# Do not turn on "Cache Everything" for HTML.
resource "cloudflare_ruleset" "cache" {
  zone_id     = var.zone_id
  name        = "Playground cache"
  description = "Cache static files, Markdown twins, discovery documents and shares; never HTML or the API"
  kind        = "zone"
  phase       = "http_request_cache_settings"

  rules {
    action      = "set_cache_settings"
    expression  = "(starts_with(http.request.uri.path, \"/api/\") or starts_with(http.request.uri.path, \"/internal/\") or http.request.uri.path in {\"/mcp\" \"/readyz\" \"/healthz\"})"
    description = "Never cache work, control and health endpoints"
    enabled     = true
    action_parameters {
      cache = false
    }
  }

  rules {
    action      = "set_cache_settings"
    expression  = "(http.request.method eq \"GET\" and http.request.uri.path in {\"/api/v1/openapi.json\" \"/api/v1/examples\" \"/api/v1/guide\" \"/api/v1/limits\" \"/openapi.json\" \"/llms.txt\" \"/.well-known/mcp.json\"})"
    description = "Discovery documents: follow the origin (5 minutes; limits 1 minute)"
    enabled     = true
    action_parameters {
      cache = true
      edge_ttl {
        mode = "respect_origin"
      }
      browser_ttl {
        mode = "respect_origin"
      }
    }
  }

  rules {
    action      = "set_cache_settings"
    expression  = "(http.request.method eq \"GET\" and starts_with(http.request.uri.path, \"/api/v1/p/\"))"
    description = "Shared playgrounds: immutable per id, but never longer than a day at the edge"
    enabled     = true
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 86400
      }
      browser_ttl {
        mode = "respect_origin"
      }
    }
  }

  rules {
    action      = "set_cache_settings"
    expression  = "(ends_with(http.request.uri.path, \".md\") and not starts_with(http.request.uri.path, \"/api/\"))"
    description = "Markdown twins of the pages: 1 hour at the edge, 5 minutes in browsers"
    enabled     = true
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 3600
      }
      browser_ttl {
        mode    = "override_origin"
        default = 300
      }
    }
  }

  rules {
    action      = "set_cache_settings"
    expression  = "(http.request.uri.path in {\"/favicon.svg\" \"/robots.txt\" \"/sitemap.xml\"})"
    description = "Static root files: 1 hour"
    enabled     = true
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 3600
      }
      browser_ttl {
        mode    = "override_origin"
        default = 3600
      }
    }
  }

  rules {
    action      = "set_cache_settings"
    expression  = "starts_with(http.request.uri.path, \"/assets/\")"
    description = "Hashed build output: a year, a deploy changes the file names"
    enabled     = true
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 31536000
      }
      browser_ttl {
        mode    = "override_origin"
        default = 31536000
      }
    }
  }
}

# Turnstile: a browser that passes it gets a signed session and the higher rate-limit tier. Nothing
# is blocked without it, so agents and curl keep working on the basic tier.
resource "cloudflare_turnstile_widget" "playground" {
  count      = var.account_id == "" ? 0 : 1
  account_id = var.account_id
  name       = "mongodb-playground"
  domains    = [var.domain, "www.${var.domain}"]
  mode       = "managed"
  region     = "world"
}

output "turnstile_site_key" {
  value = length(cloudflare_turnstile_widget.playground) > 0 ? cloudflare_turnstile_widget.playground[0].id : ""
}

output "turnstile_secret" {
  sensitive = true
  value     = length(cloudflare_turnstile_widget.playground) > 0 ? cloudflare_turnstile_widget.playground[0].secret : ""
}
