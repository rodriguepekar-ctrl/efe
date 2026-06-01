terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
  }
}

variable "region" {
  type    = string
  default = "eu-west-1"
}

variable "tenant" {
  type = string
}

variable "data_residency" {
  type    = string
  default = "EU_ONLY"
}

variable "rds_deletion_protection" {
  type    = bool
  default = false
}

variable "rds_multi_az" {
  type    = bool
  default = false
}

variable "rds_allocated_storage" {
  type    = number
  default = 20
}

variable "redis_num_cache_clusters" {
  type    = number
  default = 2
}

variable "eks_node_groups" {
  type    = any
  default = null
}

provider "aws" {
  region = var.region
}

module "efe" {
  source = "../../"

  region                      = var.region
  tenant                      = var.tenant
  data_residency              = var.data_residency
  rds_deletion_protection     = var.rds_deletion_protection
  rds_multi_az                = var.rds_multi_az
  rds_allocated_storage       = var.rds_allocated_storage
  redis_num_cache_clusters    = var.redis_num_cache_clusters
  audit_bucket_retention_days = 365   # min for tests
  enable_kms_byok             = false # cheaper for tests

  eks_node_groups = var.eks_node_groups != null ? var.eks_node_groups : {
    primary = {
      instance_types = ["t3.medium"]
      desired_size   = 2
      min_size       = 2
      max_size       = 4
      capacity_type  = "ON_DEMAND"
    }
  }
}

output "kubeconfig_command" {
  value = module.efe.kubeconfig_command
}

output "eks_cluster_name" {
  value = module.efe.eks_cluster_name
}

output "rds_endpoint" {
  value = module.efe.rds_endpoint
}

output "audit_bucket_name" {
  value = module.efe.audit_bucket_name
}
