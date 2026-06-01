# ADR-M32-002 : RRF (Reciprocal Rank Fusion) comme stratégie hybride par défaut

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 6

## Décision

`HybridMode::HybridRrf` est le mode par défaut. `HybridWeighted`
reste disponible pour les tenants qui veulent un knob explicite.

## Justification

1. **Pas de normalisation requise** — RRF opère sur les *rangs*, pas
   les scores. Les scores dense (cosine) et sparse (BM25) ont des
   échelles incomparables ; RRF élimine ce problème.
2. **Empiriquement robuste** — Cormack et al. (2009) montrent que
   RRF avec k=60 rivalise voire dépasse les fusions apprises sur
   TREC.
3. **Pas de tuning client** — l'opérateur n'a rien à choisir ; le
   default produit immédiatement des résultats de qualité.
4. **Documents-in-both reward** — la formule `Σ 1/(k + rank_r(x))`
   rewards naturellement les docs qui apparaissent dans plusieurs
   runs.

vs **weighted mean** : nécessite que le client choisisse `α` ;
sensible aux échelles ; les nouveaux clients se trompent souvent.
vs **CombSUM** : besoin de normalisation préalable.
vs **learned-to-rank** : produit une couche supplémentaire à
entraîner + maintenir.

## Conséquences

### Positives
- API one-liner : `mode: "hybrid_rrf"` et c'est tout.
- Comparable aux meilleures publis IR sans nécessiter de fine-tune.

### Négatives
- Pas de contrôle fin par le client : un client qui voudrait
  prioriser fortement la composante BM25 doit passer en
  `hybrid_weighted` avec α explicite.
