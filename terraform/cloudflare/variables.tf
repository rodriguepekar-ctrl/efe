variable "cf_api_token" {
  description = "Cloudflare API token with permissions : Zone:Edit, DNS:Edit, Workers Tunnel:Edit, Account:Read."
  type        = string
  sensitive   = true
}

variable "account_id" {
  description = "Cloudflare Account ID (32 hex chars)."
  type        = string
}

variable "zone_id" {
  description = "Cloudflare Zone ID for `efe.ai` (32 hex chars)."
  type        = string
}

variable "domain" {
  description = "Apex domain, e.g. `efe.ai`."
  type        = string
  default     = "efe.ai"
}

variable "tunnel_secret" {
  description = "Argo Tunnel secret (base64-encoded 32 random bytes)."
  type        = string
  sensitive   = true
}

variable "docs_origin" {
  description = "Origin host for docs.efe.ai (Vercel or self-hosted)."
  type        = string
  default     = "efe-docs.vercel.app"
}

variable "bot_management_enabled" {
  description = "Enable Bot Management (Enterprise plan only)."
  type        = bool
  default     = true
}
