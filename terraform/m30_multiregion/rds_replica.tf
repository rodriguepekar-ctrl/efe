# M30 — RDS Postgres cross-region read replica (eu-west-1 → eu-west-3).
#
# This is the cold-standby database for the passive region. AWS RDS
# replicates via logical decoding ; under normal load lag sits < 5 s.
# On failover the replica is promoted via `aws rds promote-read-replica`,
# becoming a standalone primary.
#
# See `docs/EFE_M30_MULTIMODAL_ARCHITECTURE.md` § 13.

resource "aws_db_instance" "replica" {
  provider                            = aws.replica
  identifier                          = "efe-pg-replica-${var.replica_region}"
  replicate_source_db                 = var.primary_db_arn
  instance_class                      = var.replica_instance_class
  allocated_storage                   = var.allocated_storage_gb
  storage_type                        = "gp3"
  iops                                = 12000
  storage_encrypted                   = true
  kms_key_id                          = aws_kms_replica_key.audit_replica.arn
  backup_retention_period             = 7
  copy_tags_to_snapshot               = true
  iam_database_authentication_enabled = true
  monitoring_interval                 = 30
  performance_insights_enabled        = true
  multi_az                            = true # within replica region
  publicly_accessible                 = false
  vpc_security_group_ids              = var.replica_security_group_ids
  db_subnet_group_name                = var.replica_subnet_group_name
  apply_immediately                   = false
  deletion_protection                 = true

  tags = merge(var.tags, {
    "efe:role"      = "passive_replica"
    "efe:milestone" = "m30"
  })
}

output "replica_endpoint" {
  value = aws_db_instance.replica.endpoint
}

output "replica_arn" {
  value = aws_db_instance.replica.arn
}
