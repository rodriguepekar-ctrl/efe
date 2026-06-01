# IRSA for efe-server (BYOK access + audit bucket write).
resource "aws_iam_policy" "efe_server_byok" {
  count       = var.enable_kms_byok ? 1 : 0
  name        = "${local.prefix}-efe-server-byok"
  description = "Allow efe-server to use BYOK KMS key for envelope encryption"

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "kms:Encrypt",
          "kms:Decrypt",
          "kms:GenerateDataKey",
          "kms:DescribeKey",
        ]
        Resource = aws_kms_key.byok[0].arn
      },
    ]
  })
}

resource "aws_iam_policy" "audit_bucket_write" {
  name        = "${local.prefix}-audit-bucket-write"
  description = "Allow efe-server to write audit chains to S3"

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:PutObjectAcl",
          "s3:AbortMultipartUpload",
          "s3:GetObject",
        ]
        Resource = "${aws_s3_bucket.audit.arn}/*"
      },
      {
        Effect   = "Allow"
        Action   = ["s3:ListBucket"]
        Resource = aws_s3_bucket.audit.arn
      },
      {
        Effect = "Allow"
        Action = [
          "kms:Encrypt",
          "kms:Decrypt",
          "kms:GenerateDataKey",
          "kms:DescribeKey",
        ]
        Resource = var.enable_kms_byok ? aws_kms_key.byok[0].arn : aws_kms_key.s3.arn
      },
    ]
  })
}

module "efe_server_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts-eks"
  version = "~> 5.39"

  role_name = "${local.prefix}-efe-server-irsa"

  role_policy_arns = merge(
    var.enable_kms_byok ? { byok = aws_iam_policy.efe_server_byok[0].arn } : {},
    {
      audit_bucket = aws_iam_policy.audit_bucket_write.arn
    }
  )

  oidc_providers = {
    main = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["efe:efe-server"]
    }
  }

  tags = local.common_tags
}
