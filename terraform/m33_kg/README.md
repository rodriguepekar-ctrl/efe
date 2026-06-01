# M33 knowledge-graph Terraform module

Provisions :

- S3 bucket for KG snapshots (per-tenant prefixed).
- Versioning + KMS encryption + lifecycle (30d → IA → Glacier).

## Usage

```hcl
module "m33_kg" {
  source       = "../m33_kg"
  bucket_name  = "efe-kg-snapshots-prod"
  kms_key_arn  = module.m30_multiregion.audit_key_arn_primary
}
```
