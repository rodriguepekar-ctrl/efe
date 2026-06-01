# `efe-stack-gcp` Terraform module

Provisions the GCP infrastructure (VPC, GKE, Cloud SQL Postgres, Memorystore Redis, KMS BYOK, GCS audit bucket, Workload Identity, Secret Manager) required to run the EFE stack on Google Cloud.

## Quick start

```hcl
module "efe_stack_gcp" {
  source = "git::https://github.com/efe-ai/efe.git//terraform/modules/gcp?ref=tf-gcp/v1.0.0"

  project_id      = "efe-acme-prod"
  region          = "europe-west1"
  tenant          = "acme"
  data_residency  = "EU_ONLY"
  enable_kms_byok = true
}
```

## Inputs

| Name | Type | Default | Description |
|------|------|---------|-------------|
| `project_id` | string | — | GCP project ID. |
| `region` | string | — | GCP region (e.g. `europe-west1`). |
| `tenant` | string | — | Tenant slug (lowercase, 3-32 chars). |
| `environment` | string | `prod` | One of `prod`, `staging`, `dev`, `poc`. |
| `data_residency` | string | `any` | `EU_ONLY` / `US_ONLY` / `any`. |
| `gke_version` | string | `1.30` | GKE Kubernetes version. |
| `gke_release_channel` | string | `REGULAR` | `RAPID` / `REGULAR` / `STABLE`. |
| `gke_master_authorized_networks` | list(string) | `[]` | Public CIDRs allowed to reach control plane. |
| `gke_node_machine_type` | string | `e2-standard-4` | Default node machine type. |
| `cloudsql_tier` | string | `db-custom-4-16384` | Cloud SQL tier. |
| `cloudsql_disk_size_gb` | number | `100` | Cloud SQL disk size. |
| `cloudsql_deletion_protection` | bool | `true` | Cloud SQL deletion protection. |
| `redis_tier` | string | `STANDARD_HA` | Memorystore tier (`BASIC` / `STANDARD_HA`). |
| `redis_memory_size_gb` | number | `4` | Memorystore memory size. |
| `enable_kms_byok` | bool | `true` | Customer-managed BYOK key. |
| `audit_bucket_retention_days` | number | `2555` | GCS audit retention (>= 365). |
| `labels` | map(string) | `{}` | Extra resource labels. |

## Outputs

| Name | Description |
|------|-------------|
| `kubeconfig_command` | gcloud command to update kubeconfig. |
| `gke_cluster_name` | GKE cluster name. |
| `gke_workload_identity_pool` | Workload Identity pool ID. |
| `cloudsql_secret_id` | Secret Manager ID with Cloud SQL credentials. |
| `redis_secret_id` | Secret Manager ID with Redis auth. |
| `byok_kms_key` | KMS BYOK key ID (null when disabled). |
| `audit_bucket_name` | GCS audit bucket name. |
| `efe_server_workload_identity_sa` | Workload Identity SA email for `efe:efe-server`. |
| `audit_hmac_secret_id` | Secret Manager ID with HMAC seed. |

## Compliance enforcement

- **Data residency** : a precondition rejects `region` mismatch (e.g. `region=us-central1` with `data_residency=EU_ONLY` fails the plan).
- **Encryption at rest** : Cloud SQL, GKE etcd, GCS audit, Secret Manager are all encrypted with CMEK when `enable_kms_byok=true`.
- **Audit retention** : GCS lifecycle enforces ≥ 1 year (default 7 years) AND a retention policy on the bucket.
- **Workload Identity** : pods authenticate to Google APIs via federated identity ; no static credentials.

## Limitations

### ⚠️ Memorystore Redis Standard does NOT support Redis Stack modules

GCP's managed Redis (Memorystore Standard) does not include the `redisearch` /
`redisjson` modules required for HNSW vector search. **Tenants enabling Mode-3**
must use the in-cluster `redis-stack` Helm sub-chart instead :

```yaml
# values-prod.yaml override for GCP tenants
redis-stack:
  enabled: true
externalRedis:
  enabled: false
```

For Memorystore-only tenants (Mode-1 / Mode-2), the in-cluster sub-chart
can be disabled.

## Examples

- `examples/basic/` — minimal config for tests / sandbox.

## Versions

- `tf-gcp/v1.0.0` (Sprint GLOSS-C initial release).
