resource "aws_elasticache_subnet_group" "efe" {
  name       = "${local.prefix}-redis-subnet"
  subnet_ids = module.vpc.database_subnets

  tags = local.common_tags
}

resource "aws_security_group" "redis" {
  name        = "${local.prefix}-redis-sg"
  description = "Allow Redis from EKS workloads only"
  vpc_id      = module.vpc.vpc_id

  ingress {
    description     = "Redis from EKS"
    from_port       = 6379
    to_port         = 6379
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }

  tags = local.common_tags
}

resource "random_password" "redis" {
  length  = 48
  special = false
}

resource "aws_cloudwatch_log_group" "redis_slow" {
  name              = "/aws/elasticache/${local.prefix}/slow"
  retention_in_days = 30
  kms_key_id        = aws_kms_key.cloudwatch.arn

  tags = local.common_tags
}

resource "aws_elasticache_replication_group" "efe" {
  replication_group_id = "${local.prefix}-redis"
  description          = "EFE Redis Stack for ${var.tenant}"

  engine         = "redis"
  engine_version = "7.1"
  node_type      = var.redis_node_type
  port           = 6379

  num_cache_clusters   = var.redis_num_cache_clusters
  parameter_group_name = "default.redis7"
  subnet_group_name    = aws_elasticache_subnet_group.efe.name
  security_group_ids   = [aws_security_group.redis.id]

  at_rest_encryption_enabled = true
  transit_encryption_enabled = true
  auth_token                 = random_password.redis.result

  automatic_failover_enabled = true
  multi_az_enabled           = var.redis_num_cache_clusters >= 2

  snapshot_retention_limit = 7
  snapshot_window          = "02:00-03:00"
  maintenance_window       = "sun:04:00-sun:05:00"

  apply_immediately = false

  log_delivery_configuration {
    destination      = aws_cloudwatch_log_group.redis_slow.name
    destination_type = "cloudwatch-logs"
    log_format       = "json"
    log_type         = "slow-log"
  }

  tags = local.common_tags
}
