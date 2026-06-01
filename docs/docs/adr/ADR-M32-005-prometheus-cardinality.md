# ADR-M32-005 : Budget cardinalité Prometheus pour M32

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 11

## Décision

Aucune métrique M32 ne porte `tenant_id` en label. Les labels :

- `distance` : {cosine, dot, l2} → 3.
- `mode`     : {flat, hnsw, hybrid_rrf, hybrid_weighted} → 4.
- `outcome`  : {ok, err} → 2.
- `model`    : {efe-classifier-768, siglip-base-768, ...} → ≤ 8.
- `collection` (gauge `_collection_size`) : ≤ 1000 actifs.

Budget total ≈ 600 séries actives.

## Justification

1. Cohérence avec M27-M31 (même politique).
2. Drill-down per-tenant via Loki + PG view
   `v_vector_collection_summary`.
3. `collection` label sur le gauge `efe_vector_collection_size`
   est acceptable car bounded par le M32.MAX_COLLECTIONS_PER_TENANT
   (10) × tenants actifs (cible 100) = 1000.

## Conséquences

### Positives
- Marge confortable Prometheus single-shard.
- Drill-down per-tenant via Loki / PG.

### Négatives
- Pas de dashboard "vector usage par tenant" en Prometheus pur.
- Reporting nécessite SQL ad-hoc.
