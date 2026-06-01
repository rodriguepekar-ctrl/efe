locals {
  prefix = "efe-${var.tenant}"

  common_tags = merge({
    "efe.io/tenant"         = var.tenant
    "efe.io/managed-by"     = "terraform"
    "efe.io/data-residency" = var.data_residency
    "efe.io/environment"    = var.environment
    "efe.io/version"        = "tf-aws-1.0.0"
  }, var.tags)
}

data "aws_region" "current" {}

data "aws_caller_identity" "current" {}

data "aws_availability_zones" "available" {
  state = "available"

  filter {
    name   = "opt-in-status"
    values = ["opt-in-not-required"]
  }
}

# Hard precondition : region must match data_residency.
resource "null_resource" "data_residency_check" {
  count = var.data_residency != "any" ? 1 : 0

  lifecycle {
    precondition {
      condition = (
        var.data_residency == "EU_ONLY"
        ? startswith(data.aws_region.current.name, "eu-")
        : startswith(data.aws_region.current.name, "us-")
      )
      error_message = "Region ${data.aws_region.current.name} does not match data_residency=${var.data_residency}."
    }
  }
}
