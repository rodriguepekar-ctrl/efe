# `efe-stack-aws` Terraform module

Provisions the AWS infrastructure (VPC, EKS, RDS Postgres, ElastiCache Redis, KMS BYOK, S3 audit bucket, IAM IRSA, Secrets Manager) required to run the EFE stack on AWS.

## Quick start

```hcl
module "efe_stack_aws" {
  source = "git::https://github.com/efe-ai/efe.git//terraform/modules/aws?ref=tf-aws/v1.0.0"

  region          = "eu-west-1"
  tenant          = "acme"
  data_residency  = "EU_ONLY"
  enable_kms_byok = true
}
```

## Inputs

| Name | Type | Default | Description |
|------|------|---------|-------------|
| `region` | string | — | AWS region (e.g. `eu-west-1`). |
| `tenant` | string | — | Tenant slug (lowercase, 3-32 chars). |
| `environment` | string | `prod` | One of `prod`, `staging`, `dev`, `poc`. |
| `vpc_cidr` | string | `10.42.0.0/16` | VPC CIDR (must be `/16`). |
| `availability_zones_count` | number | `3` | AZs to span (2-6). |
| `data_residency` | string | `any` | `EU_ONLY` / `US_ONLY` / `any`. |
| `eks_version` | string | `1.30` | EKS Kubernetes version. |
| `eks_cluster_endpoint_public_access` | bool | `false` | Public EKS API endpoint. |
| `eks_node_groups` | map(object) | (1 group) | Managed node group config. |
| `rds_instance_class` | string | `db.r6i.large` | RDS instance class. |
| `rds_allocated_storage` | number | `100` | RDS storage in GB. |
| `rds_multi_az` | bool | `true` | RDS multi-AZ. |
| `rds_backup_retention_period` | number | `14` | RDS backup retention days. |
| `rds_deletion_protection` | bool | `true` | RDS deletion protection. |
| `redis_node_type` | string | `cache.r7g.large` | ElastiCache node type. |
| `redis_num_cache_clusters` | number | `2` | Redis clusters. |
| `enable_kms_byok` | bool | `true` | Customer-managed BYOK key. |
| `audit_bucket_retention_days` | number | `2555` | S3 audit retention (>= 365). |
| `tags` | map(string) | `{}` | Extra resource tags. |

## Outputs

| Name | Description |
|------|-------------|
| `kubeconfig_command` | Command to update local kubeconfig. |
| `eks_cluster_name` | EKS cluster name. |
| `rds_secret_arn` | Secrets Manager ARN with RDS credentials. |
| `redis_secret_arn` | Secrets Manager ARN with Redis credentials. |
| `byok_kms_key_arn` | BYOK KMS key ARN (null when disabled). |
| `audit_bucket_name` | S3 audit bucket name. |
| `efe_server_irsa_role_arn` | IRSA role ARN for efe-server. |
| `audit_hmac_secret_arn` | Secrets Manager ARN with HMAC seed. |

## Compliance enforcement

- **Data residency** : a precondition rejects `region` mismatch (e.g. `region=us-east-1` with `data_residency=EU_ONLY` fails the plan).
- **Encryption at rest** : RDS, ElastiCache, EBS, S3 audit are all encrypted with KMS keys.
- **Audit retention** : S3 lifecycle enforces ≥ 1 year (default 7 years).
- **Deletion protection** : RDS deletion protection ON by default ; set `rds_deletion_protection=false` for ephemeral test environments.
- **HMAC seed** : `audit_hmac_seed` Secrets Manager entry has `lifecycle.ignore_changes` to prevent unintended rotation. Rotation requires the documented RB-04 procedure.

## Examples

- `examples/basic/` — minimal config for tests / sandbox.
- `examples/ha-production/` — full HA config for production.

## Versions

- `tf-aws/v1.0.0` (Sprint GLOSS-B initial release).
