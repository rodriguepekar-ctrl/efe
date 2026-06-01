variable "project_id" {
  description = "GCP project ID."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.project_id))
    error_message = "Project ID must match GCP naming rules ([a-z][a-z0-9-]{4,28}[a-z0-9])."
  }
}

variable "region" {
  description = "GCP region (e.g. europe-west1, us-central1)."
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
  description = "Deployment environment (prod / staging / dev / poc)."
  type        = string
  default     = "prod"

  validation {
    condition     = contains(["prod", "staging", "dev", "poc"], var.environment)
    error_message = "Must be one of: prod, staging, dev, poc."
  }
}

variable "data_residency" {
  description = "Data residency requirement (validated against region)."
  type        = string
  default     = "any"

  validation {
    condition     = contains(["EU_ONLY", "US_ONLY", "any"], var.data_residency)
    error_message = "Must be one of: EU_ONLY, US_ONLY, any."
  }
}

variable "gke_version" {
  description = "GKE Kubernetes version (master + node default)."
  type        = string
  default     = "1.30"
}

variable "gke_release_channel" {
  description = "GKE release channel (RAPID / REGULAR / STABLE)."
  type        = string
  default     = "REGULAR"

  validation {
    condition     = contains(["RAPID", "REGULAR", "STABLE"], var.gke_release_channel)
    error_message = "Must be one of: RAPID, REGULAR, STABLE."
  }
}

variable "gke_master_authorized_networks" {
  description = "Public CIDRs allowed to reach the GKE control plane (use sparingly)."
  type        = list(string)
  default     = []
}

variable "gke_node_machine_type" {
  description = "Default node machine type."
  type        = string
  default     = "e2-standard-4"
}

variable "cloudsql_tier" {
  description = "Cloud SQL machine tier (e.g. db-custom-4-16384)."
  type        = string
  default     = "db-custom-4-16384"
}

variable "cloudsql_disk_size_gb" {
  description = "Cloud SQL disk size in GB."
  type        = number
  default     = 100
}

variable "cloudsql_deletion_protection" {
  description = "Enable Cloud SQL deletion protection."
  type        = bool
  default     = true
}

variable "redis_tier" {
  description = "Memorystore tier (BASIC / STANDARD_HA)."
  type        = string
  default     = "STANDARD_HA"
}

variable "redis_memory_size_gb" {
  description = "Memorystore memory size in GB."
  type        = number
  default     = 4
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed KMS key for BYOK."
  type        = bool
  default     = true
}

variable "audit_bucket_retention_days" {
  description = "GCS audit bucket retention in days (>= 365)."
  type        = number
  default     = 2555

  validation {
    condition     = var.audit_bucket_retention_days >= 365
    error_message = "Audit bucket retention must be >= 1 year (compliance baseline)."
  }
}

variable "labels" {
  description = "Additional labels merged into common_labels."
  type        = map(string)
  default     = {}

  validation {
    condition = alltrue([
      for k, v in var.labels : can(regex("^[a-z][a-z0-9_-]{0,62}$", k))
    ])
    error_message = "Label keys must match GCP naming rules ([a-z][a-z0-9_-]{0,62})."
  }
}
