resource "azurerm_key_vault" "efe" {
  name                = "${local.prefix}-kv"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  tenant_id           = data.azurerm_client_config.current.tenant_id

  sku_name = "premium"

  enabled_for_disk_encryption     = true
  enabled_for_template_deployment = false
  enabled_for_deployment          = false
  purge_protection_enabled        = true
  soft_delete_retention_days      = 30

  enable_rbac_authorization = true

  network_acls {
    bypass         = "AzureServices"
    default_action = "Deny"
    virtual_network_subnet_ids = [
      azurerm_subnet.aks.id,
      azurerm_subnet.private_endpoints.id,
    ]
  }

  tags = local.common_tags
}

# RBAC role assignments to allow current principal to manage keys.
resource "azurerm_role_assignment" "kv_admin_self" {
  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Administrator"
  principal_id         = data.azurerm_client_config.current.object_id
}

resource "azurerm_key_vault_key" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  name         = "${local.prefix}-byok"
  key_vault_id = azurerm_key_vault.efe.id
  key_type     = "RSA"
  key_size     = 2048
  key_opts     = ["decrypt", "encrypt", "wrapKey", "unwrapKey", "sign", "verify"]

  rotation_policy {
    automatic {
      time_after_creation = "P90D"
    }
    expire_after         = "P365D"
    notify_before_expiry = "P30D"
  }

  depends_on = [azurerm_role_assignment.kv_admin_self]

  tags = local.common_tags
}

resource "azurerm_key_vault_key" "postgres" {
  count = var.enable_kms_byok ? 1 : 0

  name         = "${local.prefix}-postgres"
  key_vault_id = azurerm_key_vault.efe.id
  key_type     = "RSA"
  key_size     = 2048
  key_opts     = ["decrypt", "encrypt", "wrapKey", "unwrapKey"]

  depends_on = [azurerm_role_assignment.kv_admin_self]
}

# HMAC seed Secret.
resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "azurerm_key_vault_secret" "audit_hmac_seed" {
  name         = "${local.prefix}-audit-hmac-seed"
  value        = random_password.audit_hmac_seed.result
  key_vault_id = azurerm_key_vault.efe.id

  content_type = "audit-chain-hmac/p0"

  depends_on = [azurerm_role_assignment.kv_admin_self]

  lifecycle {
    ignore_changes = [value]
  }
}

# Postgres credentials Secret.
resource "azurerm_key_vault_secret" "postgres_credentials" {
  name = "${local.prefix}-postgres-credentials"
  value = jsonencode({
    username          = "efe_admin"
    password          = random_password.postgres.result
    host              = azurerm_postgresql_flexible_server.efe.fqdn
    port              = 5432
    dbname            = "efe"
    connection_string = "postgresql://efe_admin:${urlencode(random_password.postgres.result)}@${azurerm_postgresql_flexible_server.efe.fqdn}:5432/efe?sslmode=require"
  })
  key_vault_id = azurerm_key_vault.efe.id

  content_type = "application/json"

  depends_on = [azurerm_role_assignment.kv_admin_self]
}
