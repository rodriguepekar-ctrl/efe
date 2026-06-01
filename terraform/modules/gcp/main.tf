locals {
  prefix = "efe-${var.tenant}"

  common_labels = merge({
    "efe_io_tenant"         = var.tenant
    "efe_io_managed_by"     = "terraform"
    "efe_io_data_residency" = lower(replace(var.data_residency, "_", "-"))
    "efe_io_environment"    = var.environment
    "efe_io_version"        = "tf-gcp-1-0-0"
  }, var.labels)
}

data "google_project" "current" {
  project_id = var.project_id
}

# Hard precondition : region must match data_residency.
resource "null_resource" "data_residency_check" {
  count = var.data_residency != "any" ? 1 : 0

  lifecycle {
    precondition {
      condition = (
        var.data_residency == "EU_ONLY"
        ? can(regex("^(europe|eu)", var.region))
        : can(regex("^(us|northamerica)", var.region))
      )
      error_message = "Region '${var.region}' does not match data_residency='${var.data_residency}'."
    }
  }
}
