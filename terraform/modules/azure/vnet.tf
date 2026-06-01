resource "azurerm_virtual_network" "efe" {
  name                = "${local.prefix}-vnet"
  address_space       = [var.vnet_cidr]
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name

  tags = local.common_tags
}

resource "azurerm_subnet" "aks" {
  name                 = "${local.prefix}-aks-subnet"
  resource_group_name  = azurerm_resource_group.efe.name
  virtual_network_name = azurerm_virtual_network.efe.name
  address_prefixes     = [cidrsubnet(var.vnet_cidr, 4, 0)] # /20 from /16
}

resource "azurerm_subnet" "postgres" {
  name                 = "${local.prefix}-postgres-subnet"
  resource_group_name  = azurerm_resource_group.efe.name
  virtual_network_name = azurerm_virtual_network.efe.name
  address_prefixes     = [cidrsubnet(var.vnet_cidr, 8, 16)]

  delegation {
    name = "fs"
    service_delegation {
      name    = "Microsoft.DBforPostgreSQL/flexibleServers"
      actions = ["Microsoft.Network/virtualNetworks/subnets/join/action"]
    }
  }

  service_endpoints = ["Microsoft.Storage"]
}

resource "azurerm_subnet" "redis" {
  name                 = "${local.prefix}-redis-subnet"
  resource_group_name  = azurerm_resource_group.efe.name
  virtual_network_name = azurerm_virtual_network.efe.name
  address_prefixes     = [cidrsubnet(var.vnet_cidr, 8, 17)]
}

resource "azurerm_subnet" "private_endpoints" {
  name                 = "${local.prefix}-pe-subnet"
  resource_group_name  = azurerm_resource_group.efe.name
  virtual_network_name = azurerm_virtual_network.efe.name
  address_prefixes     = [cidrsubnet(var.vnet_cidr, 8, 18)]

  private_endpoint_network_policies = "Disabled"
}

resource "azurerm_private_dns_zone" "postgres" {
  name                = "${local.prefix}.postgres.database.azure.com"
  resource_group_name = azurerm_resource_group.efe.name

  tags = local.common_tags
}

resource "azurerm_private_dns_zone_virtual_network_link" "postgres" {
  name                  = "${local.prefix}-postgres-link"
  private_dns_zone_name = azurerm_private_dns_zone.postgres.name
  virtual_network_id    = azurerm_virtual_network.efe.id
  resource_group_name   = azurerm_resource_group.efe.name
}

resource "azurerm_log_analytics_workspace" "efe" {
  name                = "${local.prefix}-law"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  sku                 = "PerGB2018"
  retention_in_days   = 90

  tags = local.common_tags
}

resource "azurerm_network_watcher_flow_log" "efe" {
  name                      = "${local.prefix}-flowlog"
  network_security_group_id = azurerm_network_security_group.aks.id
  storage_account_id        = azurerm_storage_account.flow_logs.id
  enabled                   = true
  retention_policy {
    enabled = true
    days    = 30
  }
  network_watcher_name = "NetworkWatcher_${var.location}"
  resource_group_name  = "NetworkWatcherRG"

  traffic_analytics {
    enabled               = true
    workspace_id          = azurerm_log_analytics_workspace.efe.workspace_id
    workspace_region      = azurerm_resource_group.efe.location
    workspace_resource_id = azurerm_log_analytics_workspace.efe.id
    interval_in_minutes   = 10
  }

  tags = local.common_tags

  depends_on = [
    azurerm_storage_account.flow_logs,
  ]
}

resource "azurerm_storage_account" "flow_logs" {
  name                     = replace("${local.prefix}flowlogs", "-", "")
  resource_group_name      = azurerm_resource_group.efe.name
  location                 = azurerm_resource_group.efe.location
  account_tier             = "Standard"
  account_replication_type = "LRS"
  account_kind             = "StorageV2"
  min_tls_version          = "TLS1_2"

  https_traffic_only_enabled = true

  tags = local.common_tags
}

resource "azurerm_network_security_group" "aks" {
  name                = "${local.prefix}-aks-nsg"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name

  tags = local.common_tags
}

resource "azurerm_subnet_network_security_group_association" "aks" {
  subnet_id                 = azurerm_subnet.aks.id
  network_security_group_id = azurerm_network_security_group.aks.id
}
