# Cross-backend tolerance contracts

> **Statut** : vivant. Mis à jour à chaque sprint qui ajoute une
> implémentation alternative à une op existante.

## Pourquoi ce document

Quand plusieurs implémentations de la même opération coexistent dans
`efe-core::compute` :

* `CpuBackend` avec `matmul_naive` (triple boucle `ikj`)
* `CpuBackend` avec `matmul_blas` (via Accelerate / OpenBLAS / MKL)
* `CudaBackend` avec `matmul_cuda` (Phase 2)

…elles ne produisent **pas toujours** la même valeur au bit près. Les
ordres de réduction diffèrent (tiling BLAS vs `ikj` linéaire vs tensor
cores CUDA), et la représentation f32 absorbe des rondeurs différentes.

Ce document fixe **la tolérance acceptable** entre chaque paire de
backends, pour chaque opération. Les tests cross-backend vérifient
ces bornes ; les golden vectors les embarquent dans leur champ
`"tolerance"`.

## Notation

Pour un scalaire attendu `b` et un scalaire observé `a`, on considère
que « `a` égale `b` à tolérance près » ssi :

```
    |a − b| ≤ atol + rtol · |b|
```

* `atol` = absolute tolerance (pour les valeurs proches de 0)
* `rtol` = relative tolerance (pour les valeurs de grande magnitude)

## Table des tolérances par op (Sprint 4 Light en gras)

### Opérations atomiques

| Op | Pair comparée | `rtol` | `atol` | Justification |
|----|---|:---:|:---:|---|
| `matmul` | naive ↔ **BLAS Accelerate** | **1e-4** | **1e-5** | sgemm tile-based reorder. Borne empirique : `ε · √N ≈ 2e-6` sur N=256 ; marge ×50. |
| `matmul` | naive ↔ **BLAS OpenBLAS** | **1e-4** | **1e-5** | Identique Accelerate. |
| `matmul` | naive ↔ CUDA sgemm | 1e-4 | 1e-5 | Phase 2. |
| `matmul_fused_gelu` | naive ↔ **BLAS + fused epilog** | **1e-4** | **1e-5** | = matmul tolerance + GELU scalaire (bit-exact via libm::erff). |
| `matmul_fused_gelu` | naive ↔ CUDA kernel fused | 1e-4 | 1e-5 | Phase 2. |
| `softmax` | scalar ↔ SIMD (wide) | 1e-5 | 1e-6 | exp reste scalaire, juste addition/division vectorisées. Sprint 4 Complet. |
| `gelu` | scalar ↔ SIMD (wide) | 1e-5 | 1e-6 | erff scalaire identique, reste vectorisé. Sprint 4 Complet. |
| `cone_project` | sequential ↔ parallel (rayon) | 0 | 0 | Pas de réduction globale ; ordre par-batch préservé. Sprint 4 Complet. |
| `weighted_mean` | sequential ↔ parallel | 1e-5 | 1e-6 | Kahan par dim reste indépendant. Sprint 4 Complet. |
| `multinomial_sample` | CPU ↔ CUDA | EXACT | EXACT | Contrat d'identité sur seed fixé. Phase 2. |
| `dot` | naive Kahan ↔ BLAS sdot | 1e-5 | 1e-6 | Kahan O(ε) vs BLAS O(ε√N). |

### Opérations composées (héritent leur tolérance via composition)

Certaines ops Sprint 3 appellent `matmul` en interne. Leur tolérance
cross-backend est donc héritée — souvent **relâchée** à cause de la
composition de deux ou trois matmuls :

| Op | Composition interne | `rtol` hérité | Justification |
|----|---|:---:|---|
| `scaled_dot_product_attention` | matmul(Q, K^T) + softmax + matmul(W, V) | **2e-4** | Deux matmuls BLAS chaînés, borne cumulée `~2 · matmul_rtol`. |
| `moe_reconcile` | topk + softmax interne | **1e-5** | N'utilise PAS matmul : pas de dégradation BLAS. |
| `cone_project` | réductions par-batch | **1e-5** | N'utilise PAS matmul. |
| `weighted_mean` | réductions Kahan | **1e-5** | N'utilise PAS matmul. |
| `kl_divergence` | réductions par-axis | **1e-5** | N'utilise PAS matmul. |

**Important** : `scaled_dot_product_attention` sous `--features accelerate`
peut dériver jusqu'à `2e-4` (au lieu de `1e-5` en pure-Rust). Les tests
d'intégration SDPA qui utilisent des inputs « numériquement stables »
(zéros, uniformes, one-hot) passent à `1e-5` par coïncidence mais les
tests RANDOM doivent utiliser `2e-4`. Voir
`tests/compute_sprint4_cross_backend.rs::s4_blas_sdpa_64head_matches_manual_naive_attention`.

## Sprint 4 Light — ce qui est testé

Sprint 4 Light introduit le pair **`matmul naive ↔ BLAS`**. Les tests
croisés vivent dans :

```
crates/efe-core/tests/compute_sprint4_cross_backend.rs
```

Chaque test :
1. Construit un input aléatoire déterministe
2. Appelle `matmul` via `CpuBackend` (qui dispatche en BLAS si taille ≥ 32 768)
3. Construit la référence naïve en local (boucle `ikj` explicite)
4. Vérifie `assert_tensors_close(..., rtol=1e-4, atol=1e-5)`

Si un jour la tolérance `1e-4` s'avère trop serrée sur une taille
particulière (ex. M=1024 où `√N = 32` et les erreurs accumulent),
on relâche explicitement dans le test concerné **avec une note** et
on met à jour ce document. Pas de relâchement silencieux.

## Sprint 4 Light — ce qui n'est pas modifié

* **Golden vectors existants** : tous les JSON en `tests/golden/matmul/`
  et `tests/golden/matmul_fused_gelu/` utilisent des entrées simples
  (entiers petits). BLAS produit **les mêmes entiers exactement**.
  Les tolérances `1e-5` existantes restent valides sans changement.
* **Tests unitaires Sprint 1-3** : 1 248 tests, aucun n'est touché
  par Sprint 4 Light. Non-régression garantie par construction.

## Sprint 5 — opérations ajoutées au trait

Sprint 5 ajoute **5 opérations** au trait `ComputeBackend` (passant de
18 à 23) et **migre** ~9 modules consommateurs vers ce trait. Les
tolérances atomiques (CpuBackend pur Rust ↔ CUDA en Phase 2) suivent :

| Op (Sprint 5) | `rtol` | `atol` | Justification |
|---------------|:------:|:------:|---------------|
| `l2_distance` | `1e-5` | `1e-7` | Pure : sub + sq + sum + sqrt. Kahan-compensé. |
| `cosine_similarity_pair` | `1e-5` | `1e-7` | Pure : dot + 2 norms + divide. |
| `cosine_anchor_blend` | `1e-4` | `1e-5` | Compose : `N` × cosine + softmax-like + matmul-like. |
| `shannon_entropy` | `1e-5` | `1e-6` | Pure : Σ + transcendantale `ln`. |
| `sigmoid` | `1e-6` | `1e-7` | Pure transcendantale, branchée pour stabilité. |

## Sprint 5 — tolérances cross-module

Le refactor S5 introduit une nouvelle catégorie : la **tolérance
cross-module**. Quand la façade `els::metrics::cosine_similarity`
appelle `backend.cosine_similarity_pair`, les deux **doivent**
produire le même résultat à `1e-6` près. Ce contrat est vérifié par
les bisimulations dans `tests/compute_sprint5_cross_module.rs`.

```
                  ┌────────────────────────────────────┐
                  │   Tolérance "atomique"             │
                  │   1 op compute pure                │
                  │   rtol = 1e-5, atol = 1e-7         │
                  └────────────────────────────────────┘
                                    │ compose
                                    ▼
                  ┌────────────────────────────────────┐
                  │   Tolérance "cross-module"         │
                  │   façade = backend                 │
                  │   rtol = 1e-6, atol = 1e-7         │
                  │   (façade ne réimplémente rien)    │
                  └────────────────────────────────────┘
                                    │ compose
                                    ▼
                  ┌────────────────────────────────────┐
                  │   Tolérance "pipeline cross-mod"   │
                  │   N ops chaînées via façades       │
                  │   rtol = 2e-4, atol = 5e-5         │
                  └────────────────────────────────────┘
```

| Pipeline cross-module | rtol | atol | Test |
|-----------------------|:----:|:----:|------|
| `els::metrics::*` ↔ `backend.*` | `1e-6` | `1e-7` | `s5_els_metrics_*_matches_backend` |
| `els::projection` ↔ `backend.cosine_anchor_blend` | `1e-5` | `1e-6` | `s5_els_projection_matches_cosine_anchor_blend` |
| `genesis::dag::entropy` ↔ `backend.shannon_entropy` | `1e-5` | `1e-6` | `s5_genesis_dag_entropy_matches_backend` |
| `consensus::signals::sigmoid` ↔ `backend.sigmoid` | `1e-6` | `1e-7` | `s5_cross_module_consensus_sigmoid_score` |
| `fusion::kernels::softmax_in_place_rows` ↔ `backend.softmax` | `1e-5` | `1e-5` | (existing fusion tests) |

## Sprint 5 — contrat NaN/Inf des façades

Les façades f32 → f32 (`els::metrics::*`, `consensus::signals::sigmoid`)
ne peuvent pas surfacer un `Result` sans casser leur signature
publique. Le mapping retenu pour les `NumericalInstability` retournés
par le backend :

| Façade | Comportement legacy (pré-S5) | Comportement S5 (préservé) |
|--------|-----------------------------|---------------------------|
| `dot_product` | propage NaN | propage NaN (backend ne rejette pas) |
| `l2_norm` | propage NaN | propage NaN (backend ne rejette pas) |
| `l2_distance` | propage NaN | propage NaN (`Err` absorbé en `f32::NAN`) ⚠️ audit-S5-bis |
| `cosine_similarity` | propage NaN | retourne `0.0` (limite géométrique, plus sûr) |
| `consensus::sigmoid` | propage NaN | retourne `0.5` (limite logistique) |

Le seul cas où la façade **diverge** du legacy est `cosine_similarity`
qui passe de NaN-propagation à `0.0`. Le test
`cosine_with_nan_propagates` accepte les deux (legacy et nouveau)
explicitement, donc ce changement est documenté et non-cassant.

## Mise à jour future

Ce document s'étend quand :

* Sprint 4 Complet ajoute SIMD / parallel (lignes correspondantes
  activées pour vrai, pas juste documentées).
* Phase 2 ajoute CUDA (lignes CUDA passent de planifié à actif).
* Phase 3 ajoute mixed-precision f16/bf16 (nouvelles lignes).
* Sprint 7 introduit le bf16 dans le backend, débloquant la migration
  de `fusion::kernels::attention` et `fusion::kernels::aggregate`
  reportée par Sprint 5.

Chaque modification est un PR séparé, revuable, qui documente la
nouvelle paire de backends et la tolérance mesurée empiriquement.
