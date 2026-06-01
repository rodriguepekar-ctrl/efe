resource "aws_db_subnet_group" "efe" {
  name        = "${local.prefix}-db-subnet"
  subnet_ids  = module.vpc.database_subnets
  description = "RDS subnet group for ${local.prefix}"

  tags = local.common_tags
}

resource "aws_security_group" "rds" {
  name        = "${local.prefix}-rds-sg"
  description = "Allow Postgres from EKS workloads only"
  vpc_id      = module.vpc.vpc_id

  ingress {
    description     = "Postgres from EKS"
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }

  egress {
    description = "Multi-AZ replication egress"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.common_tags
}

resource "aws_kms_key" "rds" {
  description             = "RDS Postgres encryption for ${local.prefix}"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  tags = local.common_tags
}

resource "aws_kms_alias" "rds" {
  name          = "alias/${local.prefix}-rds"
  target_key_id = aws_kms_key.rds.key_id
}

resource "random_password" "rds" {
  length      = 48
  special     = true
  min_lower   = 4
  min_upper   = 4
  min_numeric = 4
  min_special = 4

  override_special = "!#$%&*()-_=+[]{}<>?"
}

resource "aws_db_parameter_group" "efe" {
  name   = "${local.prefix}-pg16"
  family = "postgres16"

  parameter {
    name         = "rds.logical_replication"
    value        = "1"
    apply_method = "pending-reboot"
  }

  parameter {
    name  = "log_statement"
    value = "ddl"
  }

  parameter {
    name  = "log_min_duration_statement"
    value = "1000"
  }

  parameter {
    name  = "shared_preload_libraries"
    value = "pg_stat_statements,auto_explain"
  }

  parameter {
    name  = "auto_explain.log_min_duration"
    value = "5000"
  }

  parameter {
    name  = "rds.force_ssl"
    value = "1"
  }

  tags = local.common_tags
}

resource "aws_iam_role" "rds_monitoring" {
  name = "${local.prefix}-rds-monitoring"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "monitoring.rds.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })

  tags = local.common_tags
}

resource "aws_iam_role_policy_attachment" "rds_monitoring" {
  role       = aws_iam_role.rds_monitoring.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonRDSEnhancedMonitoringRole"
}

resource "aws_db_instance" "efe" {
  identifier = "${local.prefix}-postgres"

  engine         = "postgres"
  engine_version = "16.3"
  instance_class = var.rds_instance_class

  allocated_storage     = var.rds_allocated_storage
  max_allocated_storage = var.rds_allocated_storage * 2

  storage_type      = "gp3"
  storage_encrypted = true
  kms_key_id        = aws_kms_key.rds.arn

  multi_az = var.rds_multi_az

  db_subnet_group_name   = aws_db_subnet_group.efe.name
  vpc_security_group_ids = [aws_security_group.rds.id]
  parameter_group_name   = aws_db_parameter_group.efe.name

  username = "efe_admin"
  password = random_password.rds.result
  port     = 5432

  backup_retention_period = var.rds_backup_retention_period
  backup_window           = "01:00-02:00"
  maintenance_window      = "sun:03:00-sun:04:00"
  copy_tags_to_snapshot   = true

  performance_insights_enabled          = true
  performance_insights_kms_key_id       = aws_kms_key.rds.arn
  performance_insights_retention_period = 7

  enabled_cloudwatch_logs_exports = ["postgresql", "upgrade"]

  deletion_protection       = var.rds_deletion_protection
  skip_final_snapshot       = !var.rds_deletion_protection
  final_snapshot_identifier = var.rds_deletion_protection ? "${local.prefix}-final-${formatdate("YYYYMMDDhhmmss", timestamp())}" : null

  apply_immediately          = false
  auto_minor_version_upgrade = true

  monitoring_interval = 60
  monitoring_role_arn = aws_iam_role.rds_monitoring.arn

  tags = local.common_tags

  lifecycle {
    ignore_changes = [final_snapshot_identifier]
  }
}
