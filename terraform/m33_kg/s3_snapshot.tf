# M33 — S3 bucket holding KG snapshots.
#
# Same lifecycle policy as M32 : 30d Standard → IA → Glacier-IR
# → Deep Archive. Encrypted with the M30 multi-region KMS key.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
  }
}

provider "aws" {
  region = var.region
}

resource "aws_s3_bucket" "kg_snapshots" {
  bucket        = var.bucket_name
  force_destroy = false
  tags = merge(var.tags, {
    "efe:milestone" = "m33"
    "efe:component" = "kg-snapshots"
  })
}

resource "aws_s3_bucket_versioning" "kg_snapshots" {
  bucket = aws_s3_bucket.kg_snapshots.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "kg_snapshots" {
  bucket = aws_s3_bucket.kg_snapshots.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = var.kms_key_arn
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "kg_snapshots" {
  bucket = aws_s3_bucket.kg_snapshots.id

  rule {
    id     = "retain-30-then-glacier"
    status = "Enabled"

    filter {
      prefix = var.snapshot_prefix
    }

    transition {
      days          = 30
      storage_class = "STANDARD_IA"
    }
    transition {
      days          = 90
      storage_class = "GLACIER_IR"
    }
    transition {
      days          = 365
      storage_class = "DEEP_ARCHIVE"
    }
  }
}

resource "aws_s3_bucket_public_access_block" "kg_snapshots" {
  bucket                  = aws_s3_bucket.kg_snapshots.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

output "snapshot_bucket_arn" {
  value = aws_s3_bucket.kg_snapshots.arn
}
