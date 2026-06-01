# ADR-M33-005 : Budget cardinalité Prometheus pour M33

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 9

## Décision

Aucune métrique M33 ne porte `tenant_id`. Labels :

- `kind` ∈ 6 valeurs.
- `predicate` ∈ ≤ 32 (18 base + custom bounded).
- `algo` ∈ {bfs, dfs, shortest_path}.
- `outcome` ∈ {ok, err}.
- `side` ∈ {entities, relations}.

Budget total ≈ 200 séries actives.

## Justification

Identique à M27-M32 : cardinality stable, drill-down par-tenant via
Loki + PG.
