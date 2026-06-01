# ADR-M33-008 : Entity resolution threshold (cosine 0.85 + Levenshtein 3)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 5.1

## Décision

Deux entités sont fusionnées **automatiquement** si :

- même `kind`, ET
- Levenshtein(canonical_name) ≤ 3, ET
- cosine(embedding) ≥ 0.85.

Si seul le name match (sans vector check) : surface comme
`Candidate` à l'opérateur.

## Justification

1. **Faux merges chers** — fusionner par erreur "Acme Inc." et
   "Acme Corp." casse l'attribution. Le double critère minimise.
2. **Tuned on internal data** — false-merge rate ≤ 0.5 % à ce seuil.
3. **Cosine seul insuffisant** — deux entités kind=PERSON peuvent
   avoir cosine 0.9 si elles partagent le contexte
   (e.g. "CEO de la même boîte") sans être la même personne.

vs threshold plus laxe : false-merge rate grimpe.
vs threshold plus strict : false-split rate (mêmes entities pas
mergées) grimpe ; opérateur doit corriger à la main.

## Conséquences
- Surface `Candidate` doit être audit/triagée régulièrement.
- M34 introduira un learner qui ajuste les seuils par tenant.
