# M30 — AWS KMS multi-region keys for the audit chain HMAC envelope
# and for the BYOK customer-key wrapper.
#
# Multi-region keys replicate the same key material across regions so
# that ciphertext encrypted in eu-west-1 can be decrypted in eu-west-3
# without an envelope re-wrap. This is critical for the failover RTO
# (cf. § 13.5 of the M30 architecture).

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source                = "hashicorp/aws"
      version               = "~> 5.40"
      configuration_aliases = [aws.primary, aws.replica]
    }
  }
}

# ── Primary key in eu-west-1 ──────────────────────────────────────────
resource "aws_kms_key" "audit_primary" {
  provider                 = aws.primary
  description              = "EFE M30 — audit chain HMAC key (primary)."
  customer_master_key_spec = "SYMMETRIC_DEFAULT"
  key_usage                = "ENCRYPT_DECRYPT"
  multi_region             = true
  deletion_window_in_days  = 30
  enable_key_rotation      = true

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "EnableRootPerms"
        Effect = "Allow"
        Principal = { AWS = "arn:aws:iam::${var.account_id}:root" }
        Action   = "kms:*"
        Resource = "*"
      },
      {
        Sid    = "AllowEfeAuditWorker"
        Effect = "Allow"
        Principal = { AWS = var.audit_worker_role_arn }
        Action = ["kms:Encrypt", "kms:Decrypt", "kms:GenerateDataKey", "kms:DescribeKey"]
        Resource = "*"
      }
    ]
  })

  tags = merge(var.tags, { "efe:component" = "audit", "efe:milestone" = "m30" })
}

resource "aws_kms_alias" "audit_primary" {
  provider      = aws.primary
  name          = "alias/efe-audit-key-multi-region"
  target_key_id = aws_kms_key.audit_primary.key_id
}

# ── Replica in eu-west-3 ──────────────────────────────────────────────
resource "aws_kms_replica_key" "audit_replica" {
  provider                = aws.replica
  description             = "EFE M30 — audit chain HMAC key (replica eu-west-3)."
  primary_key_arn         = aws_kms_key.audit_primary.arn
  deletion_window_in_days = 30
}

resource "aws_kms_alias" "audit_replica" {
  provider      = aws.replica
  name          = "alias/efe-audit-key-multi-region"
  target_key_id = aws_kms_replica_key.audit_replica.key_id
}

output "audit_key_arn_primary" {
  value = aws_kms_key.audit_primary.arn
}

output "audit_key_arn_replica" {
  value = aws_kms_replica_key.audit_replica.arn
}
