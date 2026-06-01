# Bucket holding S3 access logs for the audit bucket. Per AWS docs, server
# access logs cannot be encrypted with KMS (only SSE-S3 / AES256), and
# enabling versioning on a log bucket causes infinite recursion. tfsec
# warnings on this bucket are documented exceptions :
#   - aws-s3-encryption-customer-key : log buckets MUST use AES256
#   - aws-s3-enable-bucket-logging   : log-of-log would recurse
#   - aws-s3-enable-versioning       : log bucket should not be versioned
#tfsec:ignore:aws-s3-encryption-customer-key
#tfsec:ignore:aws-s3-enable-bucket-logging
#tfsec:ignore:aws-s3-enable-versioning
resource "aws_s3_bucket" "audit_access_log" {
  bucket = "${local.prefix}-audit-access-log"

  tags = merge(local.common_tags, {
    "efe.io/purpose" = "s3-access-log"
  })
}

resource "aws_s3_bucket_public_access_block" "audit_access_log" {
  bucket                  = aws_s3_bucket.audit_access_log.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

#tfsec:ignore:aws-s3-encryption-customer-key
resource "aws_s3_bucket_server_side_encryption_configuration" "audit_access_log" {
  bucket = aws_s3_bucket.audit_access_log.id

  rule {
    apply_server_side_encryption_by_default {
      # AES256 only — server access logs cannot use KMS (AWS limitation).
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "audit_access_log" {
  bucket = aws_s3_bucket.audit_access_log.id

  rule {
    id     = "expire-90d"
    status = "Enabled"

    filter {}

    expiration {
      days = 90
    }
  }
}

resource "aws_s3_bucket" "audit" {
  bucket = "${local.prefix}-audit"

  tags = merge(local.common_tags, {
    "efe.io/purpose" = "audit-archive"
  })
}

resource "aws_s3_bucket_versioning" "audit" {
  bucket = aws_s3_bucket.audit.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = var.enable_kms_byok ? aws_kms_key.byok[0].arn : aws_kms_key.s3.arn
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_public_access_block" "audit" {
  bucket                  = aws_s3_bucket.audit.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id

  rule {
    id     = "audit-retention"
    status = "Enabled"

    filter {}

    transition {
      days          = 90
      storage_class = "STANDARD_IA"
    }

    transition {
      days          = 365
      storage_class = "GLACIER"
    }

    expiration {
      days = var.audit_bucket_retention_days
    }

    noncurrent_version_expiration {
      noncurrent_days = 90
    }

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

resource "aws_s3_bucket_logging" "audit" {
  bucket = aws_s3_bucket.audit.id

  target_bucket = aws_s3_bucket.audit_access_log.id
  target_prefix = "access-logs/"
}
