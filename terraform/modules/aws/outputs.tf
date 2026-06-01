output "kubeconfig_command" {
  description = "Command to update local kubeconfig for the EKS cluster."
  value       = "aws eks update-kubeconfig --region ${var.region} --name ${module.eks.cluster_name}"
}

output "eks_cluster_name" {
  description = "EKS cluster name."
  value       = module.eks.cluster_name
}

output "eks_cluster_endpoint" {
  description = "EKS API endpoint URL."
  value       = module.eks.cluster_endpoint
  sensitive   = true
}

output "eks_oidc_provider_arn" {
  description = "OIDC provider ARN for IRSA."
  value       = module.eks.oidc_provider_arn
}

output "rds_endpoint" {
  description = "RDS Postgres endpoint hostname."
  value       = aws_db_instance.efe.address
}

output "rds_port" {
  description = "RDS Postgres port."
  value       = aws_db_instance.efe.port
}

output "rds_secret_arn" {
  description = "ARN of the Secrets Manager secret containing RDS credentials."
  value       = aws_secretsmanager_secret.rds_credentials.arn
}

output "redis_endpoint" {
  description = "ElastiCache Redis primary endpoint."
  value       = aws_elasticache_replication_group.efe.primary_endpoint_address
}

output "redis_secret_arn" {
  description = "ARN of the Secrets Manager secret containing Redis auth_token."
  value       = aws_secretsmanager_secret.redis_credentials.arn
}

output "byok_kms_key_arn" {
  description = "ARN of the customer-managed BYOK KMS key (null when disabled)."
  value       = var.enable_kms_byok ? aws_kms_key.byok[0].arn : null
}

output "audit_bucket_name" {
  description = "S3 audit bucket name."
  value       = aws_s3_bucket.audit.bucket
}

output "audit_bucket_arn" {
  description = "S3 audit bucket ARN."
  value       = aws_s3_bucket.audit.arn
}

output "efe_server_irsa_role_arn" {
  description = "IRSA role ARN for efe-server pod."
  value       = module.efe_server_irsa.iam_role_arn
}

output "vpc_id" {
  description = "VPC ID."
  value       = module.vpc.vpc_id
}

output "private_subnets" {
  description = "Private subnets IDs."
  value       = module.vpc.private_subnets
}

output "audit_hmac_secret_arn" {
  description = "ARN of the Secrets Manager secret containing the HMAC seed."
  value       = aws_secretsmanager_secret.audit_hmac_seed.arn
}
