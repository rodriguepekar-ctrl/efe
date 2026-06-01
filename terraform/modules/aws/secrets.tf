resource "aws_secretsmanager_secret" "rds_credentials" {
  name        = "${local.prefix}/rds/credentials"
  description = "RDS Postgres credentials for ${local.prefix}"
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 7

  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "rds_credentials" {
  secret_id = aws_secretsmanager_secret.rds_credentials.id

  secret_string = jsonencode({
    username          = aws_db_instance.efe.username
    password          = random_password.rds.result
    host              = aws_db_instance.efe.address
    port              = aws_db_instance.efe.port
    dbname            = "efe"
    connection_string = "postgresql://${aws_db_instance.efe.username}:${urlencode(random_password.rds.result)}@${aws_db_instance.efe.address}:${aws_db_instance.efe.port}/efe?sslmode=require"
  })
}

resource "aws_secretsmanager_secret" "redis_credentials" {
  name        = "${local.prefix}/redis/credentials"
  description = "Redis auth_token for ${local.prefix}"
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 7

  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "redis_credentials" {
  secret_id = aws_secretsmanager_secret.redis_credentials.id

  secret_string = jsonencode({
    auth_token        = random_password.redis.result
    host              = aws_elasticache_replication_group.efe.primary_endpoint_address
    port              = aws_elasticache_replication_group.efe.port
    connection_string = "rediss://:${urlencode(random_password.redis.result)}@${aws_elasticache_replication_group.efe.primary_endpoint_address}:${aws_elasticache_replication_group.efe.port}"
  })
}

resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "aws_secretsmanager_secret" "audit_hmac_seed" {
  name        = "${local.prefix}/audit/hmac-seed"
  description = "HMAC seed for D14-bis audit chain. NEVER ROTATE except via planned RB-04 procedure."
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 30

  tags = merge(local.common_tags, {
    "efe.io/purpose"     = "audit-chain-hmac"
    "efe.io/criticality" = "p0"
  })
}

resource "aws_secretsmanager_secret_version" "audit_hmac_seed" {
  secret_id     = aws_secretsmanager_secret.audit_hmac_seed.id
  secret_string = random_password.audit_hmac_seed.result

  lifecycle {
    ignore_changes = [secret_string]
  }
}
