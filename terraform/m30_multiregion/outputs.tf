output "load_balancer_id" {
  value       = cloudflare_load_balancer.api.id
  description = "Cloudflare LB resource ID (use for runbook references)."
}

output "geosteer_steering_policy" {
  value       = cloudflare_load_balancer.api.steering_policy
  description = "Active steering policy ; expected `geo`."
}
