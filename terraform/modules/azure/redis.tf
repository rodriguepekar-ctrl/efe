resource "azurerm_redis_cache" "efe" {
  name                 = "${local.prefix}-redis"
  location             = azurerm_resource_group.efe.location
  resource_group_name  = azurerm_resource_group.efe.name
  capacity             = var.redis_capacity
  family               = "P" # Premium
  sku_name             = "Premium"
  non_ssl_port_enabled = false
  minimum_tls_version  = "1.2"
  redis_version        = "6"

  public_network_access_enabled = false

  redis_configuration {
    maxmemory_policy              = "noeviction"
    rdb_backup_enabled            = true
    rdb_backup_frequency          = 60
    rdb_backup_max_snapshot_count = 3
    rdb_storage_connection_string = azurerm_storage_account.redis_backup.primary_blob_connection_string
  }

  tags = local.common_tags
}

resource "azurerm_storage_account" "redis_backup" {
  name                     = replace("${local.prefix}redisbk", "-", "")
  resource_group_name      = azurerm_resource_group.efe.name
  location                 = azurerm_resource_group.efe.location
  account_tier             = "Standard"
  account_replication_type = "LRS"
  account_kind             = "StorageV2"
  min_tls_version          = "TLS1_2"

  https_traffic_only_enabled = true

  tags = local.common_tags
}

resource "azurerm_private_endpoint" "redis" {
  name                = "${local.prefix}-redis-pe"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  subnet_id           = azurerm_subnet.private_endpoints.id

  private_service_connection {
    name                           = "${local.prefix}-redis-psc"
    private_connection_resource_id = azurerm_redis_cache.efe.id
    is_manual_connection           = false
    subresource_names              = ["redisCache"]
  }

  tags = local.common_tags
}
