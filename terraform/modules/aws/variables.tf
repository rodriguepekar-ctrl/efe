variable "region" {
  description = "AWS region for all resources."
  type        = string

  validation {
    condition     = can(regex("^[a-z]{2}-[a-z-]+-[0-9]+$", var.region))
    error_message = "Region must look like e.g. eu-west-1, us-east-2."
  }
}

variable "tenant" {
  description = "Tenant slug, used as resource prefix (e.g. 'acme')."
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

variable "vpc_cidr" {
  description = "CIDR block for the VPC. Must be /16."
  type        = string
  default     = "10.42.0.0/16"

  validation {
    condition     = can(cidrhost(var.vpc_cidr, 0)) && tonumber(split("/", var.vpc_cidr)[1]) == 16
    error_message = "vpc_cidr must be a valid /16 CIDR block."
  }
}

variable "availability_zones_count" {
  description = "Number of AZs to span (3 recommended for prod)."
  type        = number
  default     = 3

  validation {
    condition     = var.availability_zones_count >= 2 && var.availability_zones_count <= 6
    error_message = "Must be between 2 and 6."
  }
}

variable "data_residency" {
  description = "Data residency requirement. Validated against region."
  type        = string
  default     = "any"

  validation {
    condition     = contains(["EU_ONLY", "US_ONLY", "any"], var.data_residency)
    error_message = "Must be one of: EU_ONLY, US_ONLY, any."
  }
}

variable "eks_version" {
  description = "EKS Kubernetes version."
  type        = string
  default     = "1.30"
}

variable "eks_cluster_endpoint_public_access" {
  description = "Allow EKS API public access (POC only ; set false for prod)."
  type        = bool
  default     = false
}

variable "eks_cluster_endpoint_public_access_cidrs" {
  description = "CIDRs allowed to reach the public EKS API endpoint."
  type        = list(string)
  default     = []
}

variable "eks_node_groups" {
  description = "EKS managed node groups config."
  type = map(object({
    instance_types = list(string)
    desired_size   = number
    min_size       = number
    max_size       = number
    capacity_type  = string
    labels         = optional(map(string), {})
    taints = optional(list(object({
      key    = string
      value  = string
      effect = string
    })), [])
  }))
  default = {
    primary = {
      instance_types = ["m6i.xlarge"]
      desired_size   = 3
      min_size       = 3
      max_size       = 12
      capacity_type  = "ON_DEMAND"
      labels         = { role = "primary" }
    }
  }
}

variable "rds_instance_class" {
  description = "RDS Postgres instance class."
  type        = string
  default     = "db.r6i.large"
}

variable "rds_allocated_storage" {
  description = "RDS storage in GB."
  type        = number
  default     = 100

  validation {
    condition     = var.rds_allocated_storage >= 20 && var.rds_allocated_storage <= 16384
    error_message = "RDS storage must be in [20, 16384] GB."
  }
}

variable "rds_multi_az" {
  description = "Enable Multi-AZ for RDS."
  type        = bool
  default     = true
}

variable "rds_backup_retention_period" {
  description = "RDS backup retention in days."
  type        = number
  default     = 14

  validation {
    condition     = var.rds_backup_retention_period >= 7 && var.rds_backup_retention_period <= 35
    error_message = "Must be in [7, 35] days."
  }
}

variable "rds_deletion_protection" {
  description = "Enable RDS deletion protection."
  type        = bool
  default     = true
}

variable "redis_node_type" {
  description = "ElastiCache Redis node type."
  type        = string
  default     = "cache.r7g.large"
}

variable "redis_num_cache_clusters" {
  description = "Number of Redis nodes (>= 2 for failover)."
  type        = number
  default     = 2
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed KMS key for BYOK."
  type        = bool
  default     = true
}

variable "audit_bucket_retention_days" {
  description = "S3 audit bucket lifecycle retention in days (>= 365 for compliance)."
  type        = number
  default     = 2555

  validation {
    condition     = var.audit_bucket_retention_days >= 365
    error_message = "Audit bucket retention must be >= 1 year (compliance baseline)."
  }
}

variable "tags" {
  description = "Additional tags merged into common_tags."
  type        = map(string)
  default     = {}
}
