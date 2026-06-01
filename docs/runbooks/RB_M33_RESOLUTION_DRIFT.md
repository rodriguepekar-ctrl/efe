# RB_M33_RESOLUTION_DRIFT — False-merge rate climbing

**Severity** : P3 (quality, not availability).
**Trigger** : `M33EntityResolutionDrift` or tenant complaints.

## 1. Diagnose
```sql
-- Top 20 most recent merges (last 24h).
SELECT * FROM kg_query_log
 WHERE tenant_id=$1 AND op='extract' AND issued_at > NOW() - INTERVAL '24 hour'
 ORDER BY issued_at DESC LIMIT 20;
```

## 2. Common causes

### 2.1 Embedding-model drift
- A newer SigLIP version produces different embeddings ; cosines
  recompute differently.
- Mitigation : pin the model version.

### 2.2 Adversarial input
- Tenant inserted entities with near-identical names but distinct
  meaning.

## 3. Mitigation
- Lower COSINE_THRESHOLD via tenant config (out-of-scope for M33,
  see M33.1).
- Manually split via `POST /v1/kg/entities/{id}/split` (M33.1).

## 4. Post-incident
- File P3.
- If recurrent : tune COSINE_THRESHOLD per tenant.
