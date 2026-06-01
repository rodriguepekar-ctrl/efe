# `efe-stack-azure` Terraform module

Provisions Azure infrastructure (VNet, AKS, PostgreSQL Flexible Server,
Redis Cache Premium, Key Vault BYOK, Blob audit, Workload Identity,
Log Analytics) required to run the EFE stack on Azure.

## Quick start

```hcl
module "efe_stack_azure" {
  source = "git::https://github.com/efe-ai/efe.git//terraform/modules/azure?ref=tf-azure/v1.0.0"

  location        = "westeurope"
  tenant          = "acme"
  data_residency  = "EU_ONLY"
  enable_kms_byok = true
}
```

## Inputs

See `variables.tf`. Key inputs :

| Name | Type | Default | Description |
|------|------|---------|-------------|
| `location` | string | — | Azure location (e.g. `westeurope`). |
| `tenant` | string | — | Tenant slug. |
| `data_residency` | string | `any` | `EU_ONLY` / `US_ONLY` / `any`. |
| `vnet_cidr` | string | `10.42.0.0/16` | VNet CIDR. |
| `aks_version` | string | `1.30` | AKS version. |
| `aks_default_node_vm_size` | string | `Standard_D4ds_v5` | Default node SKU. |
| `postgres_sku_name` | string | `GP_Standard_D4ds_v5` | Flexible Server SKU. |
| `postgres_storage_mb` | number | `131072` | Storage size in MB. |
| `redis_capacity` | number | `1` | Premium capacity (1 ≈ 6 GB). |
| `enable_kms_byok` | bool | `true` | BYOK Key Vault key. |
| `audit_bucket_retention_days` | number | `2555` | Blob retention (>= 365). |

## Outputs

| Name | Description |
|------|-------------|
| `kubeconfig_command` | az command to update kubeconfig. |
| `aks_cluster_name` | AKS cluster name. |
| `aks_oidc_issuer_url` | OIDC issuer for Workload Identity. |
| `postgres_fqdn` | Flexible Server hostname. |
| `postgres_secret_name` | Key Vault secret with credentials. |
| `redis_hostname` | Redis Premium hostname. |
| `key_vault_name` | Key Vault name. |
| `byok_key_id` | BYOK key ID (null when disabled). |
| `audit_storage_account` | Storage Account hosting audit blob. |
| `efe_server_workload_identity_client_id` | Federated UAI client ID. |
| `audit_hmac_secret_name` | Key Vault secret name for HMAC seed. |

## Compliance enforcement

- **Data residency** : precondition rejects location mismatch.
- **Encryption at rest** : PostgreSQL Flexible Server, Storage Account, AKS secrets are CMK-encrypted when `enable_kms_byok=true`.
- **Encryption in transit** : `min_tls_version=TLS1_2` everywhere ; Postgres requires SSL.
- **Audit retention** : Blob lifecycle enforces ≥ 1 year (default 7 years).
- **Workload Identity** : pods authenticate via federated identity ; no static credentials.
- **Defender for Cloud** : enabled on AKS via Microsoft Defender add-on.

## Versions

- `tf-azure/v1.0.0` (Sprint GLOSS-C initial release).
