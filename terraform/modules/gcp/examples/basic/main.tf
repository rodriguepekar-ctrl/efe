terraform {
  required_version = ">= 1.6"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.30"
    }
  }
}

variable "project_id" {
  type = string
}

variable "region" {
  type    = string
  default = "europe-west1"
}

variable "tenant" {
  type = string
}

variable "data_residency" {
  type    = string
  default = "EU_ONLY"
}

variable "cloudsql_deletion_protection" {
  type    = bool
  default = false
}

provider "google" {
  project = var.project_id
  region  = var.region
}

module "efe" {
  source = "../../"

  project_id                   = var.project_id
  region                       = var.region
  tenant                       = var.tenant
  data_residency               = var.data_residency
  cloudsql_tier                = "db-custom-2-7680" # smaller for tests
  cloudsql_disk_size_gb        = 20
  cloudsql_deletion_protection = var.cloudsql_deletion_protection
  redis_tier                   = "BASIC"
  redis_memory_size_gb         = 1
  enable_kms_byok              = false # cheaper for tests
  audit_bucket_retention_days  = 365
}

output "kubeconfig_command" {
  value = module.efe.kubeconfig_command
}

output "gke_cluster_name" {
  value = module.efe.gke_cluster_name
}

output "audit_bucket_name" {
  value = module.efe.audit_bucket_name
}
