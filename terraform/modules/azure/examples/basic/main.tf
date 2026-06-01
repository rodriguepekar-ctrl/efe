terraform {
  required_version = ">= 1.6"

  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 3.108"
    }
  }
}

variable "location" {
  type    = string
  default = "westeurope"
}

variable "tenant" {
  type = string
}

variable "data_residency" {
  type    = string
  default = "EU_ONLY"
}

provider "azurerm" {
  features {}
}

module "efe" {
  source = "../../"

  location                    = var.location
  tenant                      = var.tenant
  data_residency              = var.data_residency
  postgres_sku_name           = "GP_Standard_D2ds_v5" # smaller for tests
  postgres_storage_mb         = 32768                 # 32 GB
  redis_capacity              = 1
  enable_kms_byok             = false # cheaper for tests
  audit_bucket_retention_days = 365
}

output "kubeconfig_command" {
  value = module.efe.kubeconfig_command
}

output "aks_cluster_name" {
  value = module.efe.aks_cluster_name
}

output "audit_storage_account" {
  value = module.efe.audit_storage_account
}
