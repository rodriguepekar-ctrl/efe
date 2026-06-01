resource "random_password" "postgres" {
  length  = 48
  special = true
}

resource "azurerm_postgresql_flexible_server" "efe" {
  name                   = "${local.prefix}-postgres"
  resource_group_name    = azurerm_resource_group.efe.name
  location               = azurerm_resource_group.efe.location
  version                = "16"
  delegated_subnet_id    = azurerm_subnet.postgres.id
  private_dns_zone_id    = azurerm_private_dns_zone.postgres.id
  administrator_login    = "efe_admin"
  administrator_password = random_password.postgres.result
  zone                   = "1"
  storage_mb             = var.postgres_storage_mb
  storage_tier           = "P30"

  sku_name = var.postgres_sku_name

  high_availability {
    mode                      = "ZoneRedundant"
    standby_availability_zone = "2"
  }

  backup_retention_days        = 14
  geo_redundant_backup_enabled = true

  dynamic "customer_managed_key" {
    for_each = var.enable_kms_byok ? [1] : []
    content {
      key_vault_key_id = azurerm_key_vault_key.postgres[0].id
    }
  }

  authentication {
    password_auth_enabled         = true
    active_directory_auth_enabled = false
  }

  tags = local.common_tags

  depends_on = [
    azurerm_private_dns_zone_virtual_network_link.postgres,
  ]
}

resource "azurerm_postgresql_flexible_server_database" "efe" {
  name      = "efe"
  server_id = azurerm_postgresql_flexible_server.efe.id
  collation = "en_US.utf8"
  charset   = "utf8"
}

resource "azurerm_postgresql_flexible_server_configuration" "log_min_duration" {
  name      = "log_min_duration_statement"
  server_id = azurerm_postgresql_flexible_server.efe.id
  value     = "1000"
}

resource "azurerm_postgresql_flexible_server_configuration" "log_statement" {
  name      = "log_statement"
  server_id = azurerm_postgresql_flexible_server.efe.id
  value     = "ddl"
}

resource "azurerm_postgresql_flexible_server_configuration" "log_connections" {
  name      = "log_connections"
  server_id = azurerm_postgresql_flexible_server.efe.id
  value     = "on"
}
