resource "azurerm_kubernetes_cluster" "efe" {
  name                = "${local.prefix}-aks"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  dns_prefix          = "${local.prefix}-dns"
  kubernetes_version  = var.aks_version

  oidc_issuer_enabled       = true # required for Workload Identity
  workload_identity_enabled = true

  default_node_pool {
    name                 = "system"
    node_count           = 3
    vm_size              = var.aks_default_node_vm_size
    vnet_subnet_id       = azurerm_subnet.aks.id
    type                 = "VirtualMachineScaleSets"
    auto_scaling_enabled = true
    min_count            = 3
    max_count            = 6

    only_critical_addons_enabled = true

    upgrade_settings {
      max_surge = "10%"
    }

    tags = local.common_tags
  }

  identity {
    type = "SystemAssigned"
  }

  network_profile {
    network_plugin      = "azure"
    network_plugin_mode = "overlay"
    network_policy      = "calico"
    service_cidr        = "10.45.0.0/16"
    dns_service_ip      = "10.45.0.10"
  }

  api_server_access_profile {
    authorized_ip_ranges = var.aks_authorized_ip_ranges
  }

  azure_active_directory_role_based_access_control {
    azure_rbac_enabled = true
  }

  oms_agent {
    log_analytics_workspace_id = azurerm_log_analytics_workspace.efe.id
  }

  microsoft_defender {
    log_analytics_workspace_id = azurerm_log_analytics_workspace.efe.id
  }

  tags = local.common_tags
}

resource "azurerm_kubernetes_cluster_node_pool" "user" {
  name                  = "user"
  kubernetes_cluster_id = azurerm_kubernetes_cluster.efe.id
  vm_size               = var.aks_default_node_vm_size
  vnet_subnet_id        = azurerm_subnet.aks.id

  auto_scaling_enabled = true
  min_count            = 3
  max_count            = 12

  upgrade_settings {
    max_surge = "10%"
  }

  tags = local.common_tags
}
