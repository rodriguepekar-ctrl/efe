# ADR-M33-006 : Caps de traversal `MAX_HOPS = 6`, `MAX_FANOUT_PER_HOP = 256`

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 5.3

## Décision

- `MAX_HOPS = 6` — couvre 99 % des use cases réels (les relations
  utiles dépassent rarement 4 hops).
- `MAX_FANOUT_PER_HOP = 256` — à 6 hops avec fan-out 256 = 256^6 =
  10^14 ⇒ infaisable ; on coupe early.

## Justification

1. **Bound combinatoire** — sans cap, un cycle ou un nœud-pivot
   (qui pointe vers 10k autres) faisait exploser la traversal.
2. **Latence prévisible** — < 10 ms p99 confirmé par bench.
3. **Operator-friendly** — un opérateur peut comprendre
   intuitivement « pourquoi mon query est lent ».

vs caps illimités → DoS-prone. vs caps plus stricts (e.g. 3 hops)
→ trop limitant pour les use cases avancés.

## Conséquences
- Les requêtes nécessitant > 6 hops doivent être décomposées par
  l'agent runner (M25).
- Fan-out cap visible dans `KgError::TraversalCapExceeded`.
