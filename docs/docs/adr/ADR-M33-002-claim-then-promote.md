# ADR-M33-002 : Modèle « claim-then-promote » plutôt qu'insertion directe

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 5.2

## Décision

Toute extraction de relation produit d'abord un `Claim`. Un Claim
n'est promu en `Relation` que si :

- ≥ 2 claims indépendants supportent le même triple, OU
- 1 claim avec confidence ≥ 0.9.

## Justification

1. **Robustesse** — un NER peut faire un faux positif occasionnel ;
   demander une corroboration limite le bruit.
2. **Provenance traçable** — chaque relation pointe sur les claims
   qui l'ont fait naître ; auditable.
3. **Annulation simple** — si une source douteuse est rétractée,
   on supprime ses claims et les relations qui n'ont plus de
   support sont automatiquement rétrogradées.

vs **promotion immédiate** : pollue le graphe avec des faux
positifs ; coût de nettoyage élevé.

## Conséquences

### Positives
- Confidence agrégée (max des claims) est honnête.
- Mécanisme de rétractation gratuit.

### Négatives
- Le graphe « live » sous-estime momentanément la couverture (une
  relation vraie mais avec un seul claim à 0.85 n'apparaît pas).
- Stockage : N claims pour M relations promues.
