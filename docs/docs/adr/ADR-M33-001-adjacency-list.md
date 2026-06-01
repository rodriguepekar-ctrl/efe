# ADR-M33-001 : Adjacency-list graph plutôt que modèle relationnel pur

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 4.1

## Décision

In-process : adjacency-list (out_edges + in_edges HashMaps).
PG mirror : tables relationnelles plates (`kg_entity`, `kg_relation`).

## Justification

1. **Performance traversal** — BFS sur adjacency-list = O(V + E) ;
   sur SQL = O(V × E) en l'absence d'index dédié. Notre cible (1 M
   nœuds, 5 M edges, 2-hop traversal) demande < 10 ms ⇒ in-process.
2. **Pas de jointures multi-table** — la couche application reste
   simple.
3. **Mirror PG pour durabilité** — quand le pod redémarre, on
   recharge depuis le mirror sans perdre de données.

vs **Neo4j / Memgraph** : SaaS / runtime à opérer, coût élevé,
overkill pour M33 cibles. Reporté à M34 si demandé.
vs **SQL pur (recursive CTE)** : O(V × E), pas viable au-delà de
100 k nœuds.

## Conséquences

### Positives
- BFS 2-hop sur 1 M nœuds = ~ 3 ms p99 (mesuré in-process).
- Bootstrap rapide depuis mirror PG.

### Négatives
- L'index tient en RAM ⇒ pression mémoire sur les pods `efe-server`.
- Replay PG → in-RAM au démarrage : ~ 30 s pour 1 M edges.
