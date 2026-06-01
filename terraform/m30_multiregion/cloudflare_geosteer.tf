# M30 — Cloudflare Geosteer (Load Balancer) — directs api.efe.ai to
# eu-west-1 by default, fails over to eu-west-3 if the EU-1 pool is
# unhealthy. The health check probes `/v1/region/health` on each
# origin.

resource "cloudflare_load_balancer_pool" "eu_west_1" {
  account_id = var.cf_account_id
  name       = "efe-eu-west-1"
  enabled    = true
  origins {
    name    = "eu-west-1-tunnel"
    address = var.eu_west_1_tunnel_hostname
    enabled = true
  }
  minimum_origins = 1
  monitor         = cloudflare_load_balancer_monitor.region_health.id
  notification_email = var.notification_email
}

resource "cloudflare_load_balancer_pool" "eu_west_3" {
  account_id = var.cf_account_id
  name       = "efe-eu-west-3"
  enabled    = true
  origins {
    name    = "eu-west-3-tunnel"
    address = var.eu_west_3_tunnel_hostname
    enabled = true
  }
  minimum_origins = 1
  monitor         = cloudflare_load_balancer_monitor.region_health.id
  notification_email = var.notification_email
}

resource "cloudflare_load_balancer_monitor" "region_health" {
  account_id  = var.cf_account_id
  type        = "https"
  expected_codes = "200"
  method      = "GET"
  timeout     = 5
  retries     = 2
  interval    = 30
  path        = "/v1/region/health"
  description = "EFE M30 — cross-region health probe"
  header {
    header = "X-EFE-Region-Peer"
    values = ["cloudflare-health-check"]
  }
}

resource "cloudflare_load_balancer" "api" {
  zone_id          = var.cf_zone_id
  name             = "api.${var.domain}"
  fallback_pool_id = cloudflare_load_balancer_pool.eu_west_1.id
  default_pool_ids = [
    cloudflare_load_balancer_pool.eu_west_1.id,
    cloudflare_load_balancer_pool.eu_west_3.id,
  ]
  proxied        = true
  steering_policy = "geo"

  # EU traffic goes to eu-west-1 by default ; failover to eu-west-3.
  region_pools {
    region   = "WEU"
    pool_ids = [cloudflare_load_balancer_pool.eu_west_1.id, cloudflare_load_balancer_pool.eu_west_3.id]
  }
  region_pools {
    region   = "EEU"
    pool_ids = [cloudflare_load_balancer_pool.eu_west_3.id, cloudflare_load_balancer_pool.eu_west_1.id]
  }

  session_affinity = "cookie"
  session_affinity_ttl = 1800
}
