# ADR-M33-007 : GraphRAG α default = 0.7 (favorise dense)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 5.4

## Décision

`α = 0.7` par défaut → 70 % de poids sur le score dense, 30 % sur
le graph-distance score.

## Justification

1. **Dense reste le signal primaire** — la requête utilisateur est
   sémantique ; les embeddings M32 capturent ça mieux que le
   simple voisinage.
2. **Graph booste les related** — le 30 % graph score est un
   re-ranking signal pour remonter les entités proches.
3. **Empirical** — sur notre dataset interne (Q&A enterprise),
   α=0.7 donne le meilleur recall@10 vs α=0.5 ou α=0.9.

vs α=0.5 (équilibre) : trop de graph noise (entities loin
sémantiquement).
vs α=0.9 (dense lourd) : graph cesse d'apporter de la valeur.

## Conséquences
- Les tenants peuvent overrider via le body de la requête.
- Future M33.1 : learned-to-rank α adapt per tenant.
