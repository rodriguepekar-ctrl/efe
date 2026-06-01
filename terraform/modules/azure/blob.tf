resource "azurerm_storage_account" "audit" {
  name                     = local.storage_name
  resource_group_name      = azurerm_resource_group.efe.name
  location                 = azurerm_resource_group.efe.location
  account_tier             = "Standard"
  account_replication_type = "GRS" # geo-redundant
  account_kind             = "StorageV2"

  min_tls_version               = "TLS1_2"
  https_traffic_only_enabled    = true
  public_network_access_enabled = false

  blob_properties {
    versioning_enabled = true

    delete_retention_policy {
      days = 7
    }

    container_delete_retention_policy {
      days = 30
    }
  }

  dynamic "customer_managed_key" {
    for_each = var.enable_kms_byok ? [1] : []
    content {
      key_vault_key_id          = azurerm_key_vault_key.byok[0].id
      user_assigned_identity_id = azurerm_user_assigned_identity.storage.id
    }
  }

  identity {
    type         = "UserAssigned"
    identity_ids = [azurerm_user_assigned_identity.storage.id]
  }

  tags = local.common_tags

  depends_on = [
    azurerm_role_assignment.storage_uai_kv,
  ]
}

resource "azurerm_storage_container" "audit" {
  name                  = "audit"
  storage_account_name  = azurerm_storage_account.audit.name
  container_access_type = "private"
}

resource "azurerm_storage_management_policy" "audit" {
  storage_account_id = azurerm_storage_account.audit.id

  rule {
    name    = "audit-retention"
    enabled = true

    filters {
      blob_types   = ["blockBlob"]
      prefix_match = ["audit/"]
    }

    actions {
      base_blob {
        tier_to_cool_after_days_since_modification_greater_than    = 90
        tier_to_archive_after_days_since_modification_greater_than = 365
        delete_after_days_since_modification_greater_than          = var.audit_bucket_retention_days
      }
    }
  }
}
