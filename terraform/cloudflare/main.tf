# M27 Commercial Edge — Cloudflare configuration.
#
# Provisions :
#   - Argo Tunnel for the origin (no public IP on K8s ingress).
#   - DNS records pointing api.efe.ai → tunnel.
#   - Cache rules for /v1/version + CDN-friendly paths.
#   - WAF managed rulesets : Cloudflare Managed + OWASP CRS.
#   - Custom WAF rules (double-encoded payloads, missing UA).
#   - Rate-limit rules for signup + anonymous classify.
#   - Bot Management (Enterprise).
#
# See `docs/EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md` § 16.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4.42"
    }
  }
}

provider "cloudflare" {
  api_token = var.cf_api_token
}

# ── Argo Tunnel ──────────────────────────────────────────────────────
resource "cloudflare_tunnel" "eu_west_1" {
  account_id = var.account_id
  name       = "efe-eu-west-1"
  secret     = var.tunnel_secret
}

resource "cloudflare_tunnel_config" "eu_west_1" {
  account_id = var.account_id
  tunnel_id  = cloudflare_tunnel.eu_west_1.id

  config {
    ingress_rule {
      hostname = "api.${var.domain}"
      service  = "http://efe-edge-envoy.efe-edge.svc.cluster.local:8443"
    }
    ingress_rule {
      service = "http_status:404"
    }
  }
}

# ── DNS records (proxied through Cloudflare) ─────────────────────────
resource "cloudflare_record" "api" {
  zone_id = var.zone_id
  name    = "api"
  type    = "CNAME"
  content = "${cloudflare_tunnel.eu_west_1.id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
  comment = "M27 — api.efe.ai → Argo Tunnel eu-west-1"
}

resource "cloudflare_record" "docs" {
  zone_id = var.zone_id
  name    = "docs"
  type    = "CNAME"
  content = var.docs_origin
  proxied = true
  ttl     = 1
  comment = "P1 marketing — Docusaurus on Vercel"
}

resource "cloudflare_record" "status" {
  zone_id = var.zone_id
  name    = "status"
  type    = "CNAME"
  content = "status.example-statuspage.com"
  proxied = false  # Don't proxy : StatusPage handles its own TLS.
  ttl     = 1
  comment = "P1 marketing — StatusPage embed"
}

# ── Cache rules ──────────────────────────────────────────────────────
resource "cloudflare_ruleset" "cache_rules" {
  zone_id     = var.zone_id
  name        = "M27 cache strategy"
  description = "Cache /v1/version + static SDK paths"
  kind        = "zone"
  phase       = "http_request_cache_settings"

  rules {
    expression = "(http.request.uri.path eq \"/v1/version\")"
    action     = "set_cache_settings"
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 300
      }
    }
    description = "Cache /v1/version 5 min"
  }

  rules {
    expression = "(http.request.uri.path matches \"^/sdk/\")"
    action     = "set_cache_settings"
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 31536000  # 1 year, SDK tarballs immutable per version
      }
    }
    description = "Cache /sdk/ artifacts forever (versioned URLs)"
  }
}

# ── WAF managed rulesets ────────────────────────────────────────────
resource "cloudflare_ruleset" "waf_managed" {
  zone_id     = var.zone_id
  name        = "M27 WAF managed"
  description = "Cloudflare Managed Ruleset + OWASP CRS"
  kind        = "zone"
  phase       = "http_request_firewall_managed"

  rules {
    action      = "execute"
    description = "Cloudflare Managed Ruleset (sensitivity = high)"
    action_parameters {
      id = "efb7b8c949ac4650a09736fc376e9aee"
    }
    expression = "true"
  }

  rules {
    action      = "execute"
    description = "OWASP Core Ruleset (paranoia = 1)"
    action_parameters {
      id = "4814384a9e5d4991b9815dcfc25d2f1f"
    }
    expression = "true"
  }
}

# ── Custom WAF rules ────────────────────────────────────────────────
resource "cloudflare_ruleset" "waf_custom" {
  zone_id     = var.zone_id
  name        = "M27 custom WAF"
  description = "Project-specific rules"
  kind        = "zone"
  phase       = "http_request_firewall_custom"

  rules {
    action      = "block"
    description = "Block double-encoded payloads (common WAF bypass)"
    expression  = "(http.request.uri contains \"%25\" and http.request.uri contains \"%\")"
  }

  rules {
    action      = "managed_challenge"
    description = "Challenge requests without User-Agent"
    expression  = "(not http.request.headers[\"user-agent\"] exists)"
  }

  rules {
    action      = "managed_challenge"
    description = "Challenge TOR exit nodes on auth paths"
    expression  = "(ip.geoip.is_tor_exit_node and http.request.uri.path matches \"^/v1/auth/\")"
  }
}

# ── Rate limit rules ────────────────────────────────────────────────
resource "cloudflare_ruleset" "rate_limit" {
  zone_id     = var.zone_id
  name        = "M27 rate limits"
  description = "Per-IP rate limits for unauthenticated paths"
  kind        = "zone"
  phase       = "http_ratelimit"

  rules {
    action = "block"
    description = "Limit signup to 5/min per IP"
    expression  = "(http.request.uri.path eq \"/v1/auth/signup\")"
    ratelimit {
      characteristics       = ["cf.colo.id", "ip.src"]
      period                = 60
      requests_per_period   = 5
      mitigation_timeout    = 86400
    }
  }

  rules {
    action      = "managed_challenge"
    description = "Challenge anonymous classify >100/min per IP"
    expression  = "(http.request.uri.path matches \"^/v1/classify\" and not http.request.headers[\"authorization\"] exists)"
    ratelimit {
      characteristics       = ["cf.colo.id", "ip.src"]
      period                = 60
      requests_per_period   = 100
      mitigation_timeout    = 600
    }
  }
}

# ── Bot Management (Enterprise plan) ────────────────────────────────
# NB : requires Enterprise plan. For Business/Pro tiers, skip this
# resource by setting var.bot_management_enabled = false.
resource "cloudflare_bot_management" "main" {
  count = var.bot_management_enabled ? 1 : 0

  zone_id                       = var.zone_id
  enable_js                     = true
  fight_mode                    = false
  suppress_session_score        = false
  auto_update_model             = true
}
