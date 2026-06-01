output "tunnel_id" {
  description = "Argo Tunnel UUID. Used by `cloudflared` DaemonSet."
  value       = cloudflare_tunnel.eu_west_1.id
}

output "tunnel_cname" {
  description = "CNAME target for api.efe.ai pointing at the tunnel."
  value       = "${cloudflare_tunnel.eu_west_1.id}.cfargotunnel.com"
}

output "api_record_name" {
  description = "Public hostname for the API."
  value       = "${cloudflare_record.api.name}.${var.domain}"
}
