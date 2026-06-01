variable "account_id" {
  description = "AWS account ID."
  type        = string
}

variable "audit_worker_role_arn" {
  description = "IAM Role ARN of the EFE audit worker (allowed to use the KMS key)."
  type        = string
}

variable "replica_region" {
  description = "AWS region for the passive replica, e.g. eu-west-3."
  type        = string
  default     = "eu-west-3"
}

variable "primary_db_arn" {
  description = "ARN of the primary RDS instance to replicate from."
  type        = string
}

variable "replica_instance_class" {
  description = "EC2 instance class for the replica."
  type        = string
  default     = "db.r6i.2xlarge"
}

variable "allocated_storage_gb" {
  description = "Allocated storage (GB) for the replica."
  type        = number
  default     = 500
}

variable "replica_security_group_ids" {
  description = "Security group IDs in the replica region's VPC."
  type        = list(string)
}

variable "replica_subnet_group_name" {
  description = "DB subnet group name in the replica region's VPC."
  type        = string
}

variable "cf_account_id" {
  description = "Cloudflare account ID."
  type        = string
}

variable "cf_zone_id" {
  description = "Cloudflare zone ID for efe.ai."
  type        = string
}

variable "domain" {
  description = "Apex domain, e.g. efe.ai."
  type        = string
  default     = "efe.ai"
}

variable "eu_west_1_tunnel_hostname" {
  description = "Hostname of the eu-west-1 Argo Tunnel origin."
  type        = string
}

variable "eu_west_3_tunnel_hostname" {
  description = "Hostname of the eu-west-3 Argo Tunnel origin."
  type        = string
}

variable "notification_email" {
  description = "Cloudflare alert recipient for LB events."
  type        = string
}

variable "tags" {
  description = "Common tags."
  type        = map(string)
  default     = {}
}
