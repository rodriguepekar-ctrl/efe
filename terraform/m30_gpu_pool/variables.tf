variable "region" {
  description = "AWS region for the GPU pool, e.g. eu-west-1."
  type        = string
}

variable "cluster_name" {
  description = "Name of the existing EKS cluster to attach the node group to."
  type        = string
}

variable "subnet_ids" {
  description = "Private subnet IDs spanning ≥ 2 AZs."
  type        = list(string)
}

variable "gpu_instance_type" {
  description = <<-EOT
    EC2 instance type for the GPU node group.

    Default `p4de.24xlarge` provides 8× A100 80GB with MIG support
    (required by the M30 architecture for 1g.10gb slices).

    Cheaper alternatives :
      - `g5.12xlarge` (A10G ×4) — staging only, NO MIG support.
      - `p4d.24xlarge` (A100 40GB ×8) — MIG supported but only
        4 × 1g.10gb slices per A100.

    Reject ARM-based GPU instances (`g5g.*`) with this module : they
    are not compatible with `ami_type = AL2_x86_64_GPU` and the
    NVIDIA GPU operator MIG configurator is x86-only.
  EOT
  type        = string
  default     = "p4de.24xlarge"

  validation {
    condition     = !startswith(var.gpu_instance_type, "g5g.")
    error_message = "g5g.* instances are ARM Graviton ; this module requires x86 (use p4de.*, p4d.*, p5.*, g5.*)."
  }
}

variable "desired_size" {
  description = "Desired number of GPU nodes at steady state."
  type        = number
  default     = 1
}

variable "min_size" {
  description = "Min number of GPU nodes (HPA shrinks down to this)."
  type        = number
  default     = 1
}

variable "max_size" {
  description = "Max number of GPU nodes (cluster autoscaler grows up to this)."
  type        = number
  default     = 4
}

variable "tags" {
  description = "Common tags applied to all resources."
  type        = map(string)
  default     = {}
}
