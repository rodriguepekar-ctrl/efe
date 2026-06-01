variable "region" {
  description = "AWS region for the snapshot bucket."
  type        = string
  default     = "eu-west-1"
}

variable "bucket_name" {
  description = "S3 bucket name. Must be globally unique."
  type        = string
}

variable "snapshot_prefix" {
  description = "Object key prefix for snapshots (e.g. `vector/`)."
  type        = string
  default     = "vector/"
}

variable "kms_key_arn" {
  description = "KMS multi-region key ARN (re-use the M30 audit key)."
  type        = string
}

variable "tags" {
  description = "Common tags."
  type        = map(string)
  default     = {}
}
