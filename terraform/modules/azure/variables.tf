variable "location" {
  description = "Azure location (e.g. westeurope, eastus)."
  type        = string
}

variable "tenant" {
  description = "Tenant slug, used as resource prefix."
  type        = string

  validation {
    condition     = can(regex("^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$", var.tenant))
    error_message = "Tenant slug must match [a-z0-9][a-z0-9-]{1,30}[a-z0-9]."
  }
}

variable "environment" {
  description = "Deployment environment."
  type        = string
  default     = "prod"

  validation {
    condition     = contains(["prod", "staging", "dev", "poc"], var.environment)
    error_message = "Must be one of: prod, staging, dev, poc."
  }
}

variable "data_residency" {
  description = "Data residency requirement (validated against location)."
  type        = string
  default     = "any"

  validation {
    condition     = contains(["EU_ONLY", "US_ONLY", "any"], var.data_residency)
    error_message = "Must be one of: EU_ONLY, US_ONLY, any."
  }
}

variable "vnet_cidr" {
  description = "VNet CIDR block."
  type        = string
  default     = "10.42.0.0/16"
}

variable "aks_version" {
  description = "AKS Kubernetes version."
  type        = string
  default     = "1.30"
}

variable "aks_authorized_ip_ranges" {
  description = "Public IP ranges allowed to reach AKS control plane."
  type        = list(string)
  default     = []
}

variable "aks_default_node_vm_size" {
  description = "Default AKS node VM size."
  type        = string
  default     = "Standard_D4ds_v5"
}

variable "postgres_sku_name" {
  description = "PostgreSQL Flexible Server SKU."
  type        = string
  default     = "GP_Standard_D4ds_v5"
}

variable "postgres_storage_mb" {
  description = "PostgreSQL Flexible Server storage in MB."
  type        = number
  default     = 131072 # 128 GB

  validation {
    condition     = var.postgres_storage_mb >= 32768 && var.postgres_storage_mb <= 16777216
    error_message = "Storage must be in [32 GB, 16 TB]."
  }
}

variable "redis_capacity" {
  description = "Redis Premium capacity (1 = ~6 GB, 2 = ~13 GB, ...)."
  type        = number
  default     = 1
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed Key Vault key for BYOK."
  type        = bool
  default     = true
}

variable "audit_bucket_retention_days" {
  description = "Blob audit retention in days (>= 365)."
  type        = number
  default     = 2555

  validation {
    condition     = var.audit_bucket_retention_days >= 365
    error_message = "Audit retention must be >= 1 year (compliance baseline)."
  }
}

variable "tags" {
  description = "Additional tags merged into common_tags."
  type        = map(string)
  default     = {}
}
