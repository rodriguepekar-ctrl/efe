# M32 vector-snapshot Terraform module

Provisions :

- **S3 bucket** for vector-index snapshots.
- Versioning + KMS encryption (re-uses the M30 multi-region key).
- Lifecycle : Standard 30 d → Standard-IA → Glacier-IR (90 d) →
  Deep Archive (365 d).
- Public-access block.

## Usage

```hcl
module "m32_vector" {
  source       = "../m32_vector"
  region       = "eu-west-1"
  bucket_name  = "efe-vector-snapshots-prod"
  kms_key_arn  = module.m30_multiregion.audit_key_arn_primary
  tags         = { env = "prod" }
}
```

## Outputs

| Output | Description |
|---|---|
| `snapshot_bucket_arn`  | Bucket ARN. |
| `snapshot_bucket_name` | Bucket name (for IAM policies). |
