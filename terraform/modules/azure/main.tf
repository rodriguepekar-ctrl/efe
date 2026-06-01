locals {
  prefix       = "efe-${var.tenant}"
  storage_name = replace("${local.prefix}audit", "-", "")

  common_tags = merge({
    "efe.io/tenant"         = var.tenant
    "efe.io/managed-by"     = "terraform"
    "efe.io/data-residency" = var.data_residency
    "efe.io/environment"    = var.environment
    "efe.io/version"        = "tf-azure-1.0.0"
  }, var.tags)
}

data "azurerm_client_config" "current" {}

resource "azurerm_resource_group" "efe" {
  name     = "${local.prefix}-rg"
  location = var.location

  tags = local.common_tags
}

# Hard precondition : location must match data_residency.
resource "null_resource" "data_residency_check" {
  count = var.data_residency != "any" ? 1 : 0

  lifecycle {
    precondition {
      condition = (
        var.data_residency == "EU_ONLY"
        ? can(regex("^(west|north|france|germany|switzerland|sweden|norway|italy|uk|swedencentral)", lower(var.location)))
        : can(regex("^(east|west|central|south|north).*us$", lower(var.location))) || can(regex("^(eastus|westus|centralus|southcentralus|northcentralus|eastus2|westus2|westus3)", lower(var.location)))
      )
      error_message = "Location '${var.location}' does not match data_residency='${var.data_residency}'."
    }
  }
}
