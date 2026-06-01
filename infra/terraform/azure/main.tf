// Sprint Φ-D-LATACT D93 — Azure module : 3-cloud parity (= match AWS + GCP).
// Cf. docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md § 8.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 3.80"
    }
    // L1 audit fix Φ-D-LATACT.L1-4 : random provider required for
    // random_password.pg below. Pre-fix `terraform init` would fail
    // with "no provider for resource random_password".
    random = {
      source  = "hashicorp/random"
      version = "~> 3.5"
    }
  }
}

provider "azurerm" {
  features {}
}

variable "tenant_id" {
  type        = string
  description = "EFE tenant identifier (= scope for this Azure stack)"
}

variable "location" {
  type    = string
  default = "westeurope"
}

variable "aks_node_count" {
  type    = number
  default = 3
}

variable "aks_vm_size" {
  type    = string
  default = "Standard_D4s_v3"   // = 4 vCPU, 16 GB RAM (parité AWS m5.xlarge / GCP n2-standard-4)
}

// ─── Resource Group ──────────────────────────────────────────────
resource "azurerm_resource_group" "efe" {
  name     = "rg-efe-${var.tenant_id}"
  location = var.location
  tags = {
    tenant  = var.tenant_id
    sprint  = "Phi-D-LATACT"
    purpose = "efe-self-service-poc"
  }
}

// ─── AKS Cluster (= match EKS / GKE) ─────────────────────────────
resource "azurerm_kubernetes_cluster" "efe" {
  name                = "aks-efe-${var.tenant_id}"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  dns_prefix          = "efe-${var.tenant_id}"

  default_node_pool {
    name       = "default"
    node_count = var.aks_node_count
    vm_size    = var.aks_vm_size
  }

  identity {
    type = "SystemAssigned"
  }

  tags = { tenant = var.tenant_id }
}

// ─── PostgreSQL Flexible Server (= match RDS / CloudSQL) ─────────
resource "azurerm_postgresql_flexible_server" "efe" {
  name                = "pg-efe-${var.tenant_id}"
  resource_group_name = azurerm_resource_group.efe.name
  location            = azurerm_resource_group.efe.location
  version             = "16"
  administrator_login = "efeadmin"
  administrator_password = random_password.pg.result

  sku_name = "B_Standard_B2s"   // parité avec AWS db.t3.medium
  storage_mb = 32768            // 32 GB
  backup_retention_days = 7
  zone = "1"

  tags = { tenant = var.tenant_id }
}

resource "random_password" "pg" {
  length  = 32
  special = true
}

// ─── Azure Cache for Redis (= match ElastiCache) ─────────────────
resource "azurerm_redis_cache" "efe" {
  name                = "redis-efe-${var.tenant_id}"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  capacity            = 1
  family              = "C"
  sku_name            = "Standard"
  minimum_tls_version = "1.2"

  tags = { tenant = var.tenant_id }
}

// ─── Storage Account (= match S3 / GCS) ─────────────────────────
resource "azurerm_storage_account" "efe" {
  name                     = replace("st${var.tenant_id}", "-", "")
  resource_group_name      = azurerm_resource_group.efe.name
  location                 = azurerm_resource_group.efe.location
  account_tier             = "Standard"
  account_replication_type = "GRS"
  min_tls_version          = "TLS1_2"
  allow_nested_items_to_be_public = false

  tags = { tenant = var.tenant_id }
}

// ─── Key Vault (= match Secrets Manager / Secret Manager) ────────
resource "azurerm_key_vault" "efe" {
  name                = "kv-efe-${var.tenant_id}"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  tenant_id           = data.azurerm_client_config.current.tenant_id
  sku_name            = "standard"

  network_acls {
    default_action = "Deny"
    bypass         = "AzureServices"
  }

  tags = { tenant = var.tenant_id }
}

data "azurerm_client_config" "current" {}

// ─── Outputs ─────────────────────────────────────────────────────
output "aks_kube_config" {
  value     = azurerm_kubernetes_cluster.efe.kube_config_raw
  sensitive = true
}

output "pg_connection_string" {
  value     = "postgresql://${azurerm_postgresql_flexible_server.efe.administrator_login}:${random_password.pg.result}@${azurerm_postgresql_flexible_server.efe.fqdn}:5432/efe"
  sensitive = true
}

output "redis_hostname" {
  value = azurerm_redis_cache.efe.hostname
}

output "storage_account_name" {
  value = azurerm_storage_account.efe.name
}
