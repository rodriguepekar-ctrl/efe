output "kubeconfig_command" {
  description = "Command to update local kubeconfig for the AKS cluster."
  value       = "az aks get-credentials --resource-group ${azurerm_resource_group.efe.name} --name ${azurerm_kubernetes_cluster.efe.name}"
}

output "aks_cluster_name" {
  value = azurerm_kubernetes_cluster.efe.name
}

output "aks_oidc_issuer_url" {
  value = azurerm_kubernetes_cluster.efe.oidc_issuer_url
}

output "postgres_fqdn" {
  value = azurerm_postgresql_flexible_server.efe.fqdn
}

output "postgres_secret_name" {
  description = "Key Vault secret containing Postgres credentials."
  value       = azurerm_key_vault_secret.postgres_credentials.name
}

output "redis_hostname" {
  value = azurerm_redis_cache.efe.hostname
}

output "key_vault_name" {
  value = azurerm_key_vault.efe.name
}

output "byok_key_id" {
  description = "Key Vault BYOK key ID (null when disabled)."
  value       = var.enable_kms_byok ? azurerm_key_vault_key.byok[0].id : null
}

output "audit_storage_account" {
  value = azurerm_storage_account.audit.name
}

output "efe_server_workload_identity_client_id" {
  description = "User-Assigned Managed Identity client ID for efe-server pod."
  value       = azurerm_user_assigned_identity.efe_server.client_id
}

output "audit_hmac_secret_name" {
  value = azurerm_key_vault_secret.audit_hmac_seed.name
}

output "resource_group_name" {
  value = azurerm_resource_group.efe.name
}
