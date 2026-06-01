resource "azurerm_user_assigned_identity" "efe_server" {
  name                = "${local.prefix}-efe-server-uai"
  resource_group_name = azurerm_resource_group.efe.name
  location            = azurerm_resource_group.efe.location

  tags = local.common_tags
}

resource "azurerm_federated_identity_credential" "efe_server" {
  name                = "${local.prefix}-efe-server-fic"
  resource_group_name = azurerm_resource_group.efe.name
  parent_id           = azurerm_user_assigned_identity.efe_server.id
  audience            = ["api://AzureADTokenExchange"]
  issuer              = azurerm_kubernetes_cluster.efe.oidc_issuer_url
  subject             = "system:serviceaccount:efe:efe-server"
}

# efe-server can use BYOK key.
resource "azurerm_role_assignment" "efe_server_kv_user" {
  count = var.enable_kms_byok ? 1 : 0

  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Crypto User"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}

# efe-server can read HMAC seed + Postgres credentials.
resource "azurerm_role_assignment" "efe_server_kv_secrets" {
  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Secrets User"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}

# efe-server can write to audit blob.
resource "azurerm_role_assignment" "efe_server_blob" {
  scope                = azurerm_storage_container.audit.resource_manager_id
  role_definition_name = "Storage Blob Data Contributor"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}

# Storage account UAI for CMK encryption (mandatory).
resource "azurerm_user_assigned_identity" "storage" {
  name                = "${local.prefix}-storage-uai"
  resource_group_name = azurerm_resource_group.efe.name
  location            = azurerm_resource_group.efe.location

  tags = local.common_tags
}

resource "azurerm_role_assignment" "storage_uai_kv" {
  count = var.enable_kms_byok ? 1 : 0

  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Crypto Service Encryption User"
  principal_id         = azurerm_user_assigned_identity.storage.principal_id
}
