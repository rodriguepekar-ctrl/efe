# M32 — S3 bucket holding vector-index snapshots.
#
# One bucket per region. Lifecycle rule retains the last 30
# snapshots ; older ones are transitioned to Glacier Deep Archive
# for SOC 2 BCDR evidence.

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

resource "aws_s3_bucket" "snapshots" {
  bucket        = var.bucket_name
  force_destroy = false
  tags = merge(var.tags, {
    "efe:milestone" = "m32"
    "efe:component" = "vector-snapshots"
  })
}

resource "aws_s3_bucket_versioning" "snapshots" {
  bucket = aws_s3_bucket.snapshots.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "snapshots" {
  bucket = aws_s3_bucket.snapshots.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = var.kms_key_arn
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "snapshots" {
  bucket = aws_s3_bucket.snapshots.id

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

resource "aws_s3_bucket_public_access_block" "snapshots" {
  bucket                  = aws_s3_bucket.snapshots.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

output "snapshot_bucket_arn" {
  value = aws_s3_bucket.snapshots.arn
}

output "snapshot_bucket_name" {
  value = aws_s3_bucket.snapshots.id
}
