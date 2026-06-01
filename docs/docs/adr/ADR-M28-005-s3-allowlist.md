# ADR-M28-005 : S3 bucket allow-list per tenant (no cross-account role)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md § 8.4

## Contexte

Le Batch API accepte un `input_url` + `output_url` au format `s3://`.
Trois approches :
- (A) EFE assume une IAM role dans le compte AWS du customer.
- (B) Le customer fournit des STS credentials short-lived.
- (C) Le customer ajoute son bucket à un EFE allow-list + EFE écrit
      avec ses propres creds (cross-account bucket policy côté
      customer).

## Décision

**Option C — allow-list + cross-account bucket policy**.

Justification :
1. **Pas de role assumption** : EFE ne demande pas accès IAM côté
   customer. Plus simple à provisioner, audit-friendly.
2. **Customer-controlled** : seuls les buckets explicitement listés
   sont touchés. Pas de path traversal vers des resources internes.
3. **Audit trail PG** : `tenant_s3_allowlist` est la source de vérité.

## Conséquences

### Positives
- Setup customer : ajouter une bucket policy `Allow s3:GetObject,PutObject`
  à efe-batch-worker IAM principal, puis `POST /v1/admin/s3-allow`.
- EFE ne stocke aucune credential customer.

### Négatives
- Pour les customers qui veulent EFE-managed S3 (CSP-managed encryption),
  on offrira un upgrade Enterprise tier (EFE provisions a bucket per
  tenant). M29.
