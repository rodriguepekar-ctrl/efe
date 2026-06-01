# ADR-M33-003 : Graphe in-process + mirror PG plutôt que Neo4j / Memgraph

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 4

## Décision

L'index graphe vit dans le processus `efe-server`. Postgres
mirror = source de vérité.

## Justification

1. Cohérence avec M32 (vector search in-process).
2. Pas de nouveau service à opérer/monitorer.
3. Coût Neo4j Enterprise très élevé.
4. Performance : 2-hop sur 1 M nœuds < 10 ms in-process.

## Conséquences
- Pression RAM identique au M32 vector.
- M34 réintroduira Neo4j out-of-process pour les tenants > 10 M edges.
