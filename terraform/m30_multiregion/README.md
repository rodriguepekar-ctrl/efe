# M30 multi-region Terraform module

Provisions :

- **KMS multi-region keys** for audit-chain HMAC (`kms.tf`).
- **RDS cross-region read replica** in `eu-west-3` from the
  `eu-west-1` primary (`rds_replica.tf`).
- **Cloudflare Geosteer Load Balancer** with `eu-west-1` as
  default pool and `eu-west-3` as fallback (`cloudflare_geosteer.tf`).

## Usage

```hcl
provider "aws" {
  alias  = "primary"
  region = "eu-west-1"
}
provider "aws" {
  alias  = "replica"
  region = "eu-west-3"
}

module "m30" {
  source = "../m30_multiregion"
  providers = {
    aws.primary = aws.primary
    aws.replica = aws.replica
  }
  account_id                    = var.account_id
  audit_worker_role_arn         = module.iam.audit_worker_role_arn
  primary_db_arn                = module.rds_primary.arn
  replica_security_group_ids    = module.vpc_eu_west_3.private_sg_ids
  replica_subnet_group_name     = module.vpc_eu_west_3.db_subnet_group_name
  cf_account_id                 = var.cf_account_id
  cf_zone_id                    = var.cf_zone_id
  eu_west_1_tunnel_hostname     = "tunnel.eu-west-1.efe.ai"
  eu_west_3_tunnel_hostname     = "tunnel.eu-west-3.efe.ai"
  notification_email            = "ops@efe.ai"
}
```

## Outputs

| Output                  | Description                                       |
|-------------------------|---------------------------------------------------|
| `audit_key_arn_primary` | KMS multi-region primary key ARN (eu-west-1).     |
| `audit_key_arn_replica` | KMS multi-region replica key ARN (eu-west-3).     |
| `replica_endpoint`      | RDS replica endpoint (passive DB).                |
| `load_balancer_id`      | Cloudflare LB resource ID.                        |
