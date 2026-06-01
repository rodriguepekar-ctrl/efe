# Sprint 5 — Refactor `ComputeBackend` cross-modules
## Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture Sprint 5
**Branche thématique** : Compute Phase 1 — sortie post-Sprint 4 Light
**Sprint cible** : S5 (post-S4L, pré-Phase 2 CUDA)
**Auteur** : audit & design — `rodrigue.pekar@icloud.com`
**Dépend de** : `crates/efe-core/src/compute/ROADMAP.md`,
`docs/compute/sprint3_math.md`, `docs/compute/tolerances.md`,
`M5_ARCHITECTURE.md` (Fusion Kernels)
**Consommé par** : tous les modules `efe-core/src/{alignment,consensus,
dissolution,fusion,genesis,els,collapse}` ainsi que les tests E2E
existants.
**Statut** : spec gelée pour implémentation — aucune ambiguïté
résiduelle après audit cross-module exhaustif.

> **Mission** : faire converger toutes les opérations numériques
> dispersées dans `efe-core` vers le trait `ComputeBackend`
> (Sprint 1-4), de sorte que (1) la Phase 2 CUDA bascule l'ensemble
> de la plateforme d'un seul coup, (2) les tolérances cross-backend
> soient unifiées, et (3) chaque NaN/Inf devienne une erreur
> structurée plutôt qu'un piège silencieux.

---

## Table des matières

1. [Mission, justification et critères de sortie](#1-mission-justification-et-critères-de-sortie)
2. [État AS-IS — la dette numérique cartographiée](#2-état-as-is--la-dette-numérique-cartographiée)
3. [État TO-BE — vision cible](#3-état-to-be--vision-cible)
4. [Inventaire chirurgical des duplications](#4-inventaire-chirurgical-des-duplications)
5. [Extensions requises au trait `ComputeBackend`](#5-extensions-requises-au-trait-computebackend)
6. [Matrice de dépendances inter-modules](#6-matrice-de-dépendances-inter-modules)
7. [Plan de migration en 9 phases (A1–A9)](#7-plan-de-migration-en-9-phases-a1a9)
8. [Stratégie de test et goldens cross-module](#8-stratégie-de-test-et-goldens-cross-module)
9. [Contrats de tolérance unifiés](#9-contrats-de-tolérance-unifiés)
10. [Critères d'acceptation par phase](#10-critères-dacceptation-par-phase)
11. [Plan jour-par-jour Sprint 5](#11-plan-jour-par-jour-sprint-5)
12. [Risques et mitigations](#12-risques-et-mitigations)
13. [Rollback et compatibilité ascendante](#13-rollback-et-compatibilité-ascendante)
14. [Préparation Phase 2 CUDA](#14-préparation-phase-2-cuda)
15. [Annexes](#15-annexes)

---

## 1. Mission, justification et critères de sortie

### 1.1 Mission

Refactorer `efe-core` pour que **toutes** les opérations numériques
**substantielles** transitent par le trait `ComputeBackend`. À l'issue
du Sprint 5 :

* Aucun module hors `compute/` ne contient de boucle `for` qui calcule
  un produit scalaire, une norme, un softmax, une attention, une
  projection ou une entropie.
* Chaque tensor numérique substantiel est un `Tensor<f32>` ou son
  équivalent `bf16` exposé par `compute::tensor`.
* Chaque cas dégénéré (NaN, Inf, divisions par zéro, somme nulle)
  produit `Err(NumericalInstability)` ou `Err(ShapeMismatch)`, jamais
  un `f32::NAN` silencieux propagé.
* Les contrats de tolérance cross-backend documentés dans
  `tolerances.md` deviennent **uniformes** — un test cross-module
  utilise la même rtol/atol qu'un test cross-backend.

### 1.2 Justification stratégique (4 raisons)

1. **Critère de sortie Phase 1 explicite**. Le ROADMAP du module
   `compute/` (ligne 645-647) liste comme critère de sortie de
   Phase 1 : « `efe-core/src/alignment/, consensus/, dissolution/,
   fusion/, genesis/` refactorés pour passer par `ComputeBackend`
   (PR de migration séparée en post-S4) ». Sauter ce critère, c'est
   laisser Phase 1 ouverte.
2. **Phase 2 CUDA repose dessus**. Tant qu'`els::projection` réimplémente
   son propre softmax-cosine-pondéré, basculer le backend vers CUDA
   ne propage pas l'accélération. Le refactor matérialise la promesse
   « swap `CpuBackend` → `CudaBackend` et tout devient GPU ».
3. **L'audit 5 a montré la chasse aux NaN silencieux est rentable**.
   Sur les seules 18 opérations de `compute/`, 5 audits ont trouvé
   3 bugs (NaN propagation, golden fictifs, dead_code stale). Les
   ~15 implémentations artisanales dispersées dans le reste de
   `efe-core` n'ont **jamais été auditées** sous le même angle.
4. **Réduction de surface de maintenance**. ~1 200 LOC de code
   numérique artisanal vont être supprimés, remplacés par ~200 LOC
   de glue. Chaque kernel CUDA porté en Phase 2 sert immédiatement
   tous les call sites.

### 1.3 Critères de sortie Sprint 5

| # | Critère | Mesure |
|---|---------|:------:|
| C1 | 100 % des opérations numériques substantielles passent par `ComputeBackend` | Audit `grep -r "powi\|sqrt\|.exp(" crates/efe-core/src/{alignment,consensus,dissolution,fusion,genesis,els,collapse}` ne retourne que les call sites de `*_backend.cosine_*` ou similaires |
| C2 | 0 implémentation artisanale de softmax/attention/cosine/dot/l2_norm/entropy hors `compute/` | Liste blanche figée dans le test `s5_no_artisanal_numeric_ops` (parse l'arbre source) |
| C3 | 100 % des NaN dégénérés produisent une `Err` structurée | Tests proptest avec inputs adversariaux (NaN, Inf, négatifs) |
| C4 | Tests verts complets : `cargo test -p efe-core` ≥ 1 350 (vs 1 276 audit 5) | `cargo test` |
| C5 | Tests cross-module : 12 nouveaux tests vérifiant que la migration n'a pas dérivé numériquement | `tests/compute_sprint5_cross_module.rs` |
| C6 | Goldens cross-module : 6 nouveaux JSON couvrant chaque chaîne migrée | `tests/golden/cross_module/*.json` |
| C7 | Doc `tolerances.md` étendue avec section « cross-module tolerances » | `docs/compute/tolerances.md` |
| C8 | Aucune régression de performance &gt; 5 % sur les benchmarks existants | `cargo bench -- --baseline pre-s5` |
| C9 | Clippy strict (`-D warnings`) en pur Rust ET avec `--features accelerate` | CI |
| C10 | ROADMAP.md mis à jour : Phase 1 cochée comme livrée | `git diff ROADMAP.md` |

---

## 2. État AS-IS — la dette numérique cartographiée

### 2.1 Schéma global de la dette

```
                              compute/  ←─── 18 ops Sprint 1-3 propres
                                  ▲                     │
                                  │                     │ (jamais consommé
                                  │                     │  par les modules
                                  │                     ▼  ci-dessous)
       ┌────────────────────────────────────────────────────────┐
       │                                                        │
   (jamais utilisé)                       implémentations artisanales
       │                                                        │
   ┌───┴────┐  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌────┴────┐
   │ ELS    │  │ Genesis  │  │ Alignment │  │ Fusion   │  │ Consensus│
   │ metrics│  │ meta_an. │  │ policy    │  │ kernels/ │  │ signals  │
   │ project│  │ dag.rs   │  │ compose   │  │ {softmax,│  │          │
   │ stats  │  │ cardin.  │  │           │  │ attention│  │          │
   │        │  │          │  │           │  │ entropy, │  │          │
   │        │  │          │  │           │  │ aggregate│  │          │
   └────────┘  └──────────┘  └───────────┘  │ , …}     │  └─────────┘
                                            └──────────┘
```

### 2.2 Sept duplications principales identifiées

| # | Op artisanale | Fichiers concernés | Lignes |
|--:|---------------|:-------------------|:------:|
| 1 | `cosine_similarity` (a, b) | `els/metrics.rs:79`, `genesis/meta_analyzer.rs:138` (**duplicat exact**), `alignment/policy.rs:267-274`, `alignment/compose.rs:130-144` | ~80 |
| 2 | `l2_norm` / `l2_distance` | `els/metrics.rs:46,61`, `dissolution/alignment.rs:121`, `els/projection.rs:90,140`, `consensus/signals.rs:171,175` | ~120 |
| 3 | `dot_product` (a, b) | `els/metrics.rs:34`, déjà dans `compute::dot` | ~30 |
| 4 | `softmax` artisanal | `fusion/kernels/softmax.rs` (243 LOC), `genesis/meta_analyzer.rs:185-200` | ~270 |
| 5 | `scaled_dot_product_attention` artisanal | `fusion/kernels/attention.rs` (267 LOC) | ~270 |
| 6 | Entropie de Shannon | `fusion/kernels/entropy.rs` (215 LOC), `genesis/dag.rs:192,307`, `genesis/cardinality.rs:155,186` | ~250 |
| 7 | Sigmoid `1/(1+exp(-x))` | `consensus/signals.rs:240-246` | ~10 |

**Total dette numérique cartographiée : ~1 030 LOC** dispersés sur 7 modules.

### 2.3 Cas particulier — la « projection cosine-pondérée »

Une **8ème duplication** mérite mention séparée car elle n'est pas
strictement redondante :

```rust
// els/projection.rs — project_vector_onto_anchors
//   1. weights[i] = max(0, cos(v, a_i))      — positive cosines only
//   2. centroid = Σ_i (w_i / Σw) · a_i        — convex combination
//   3. result = (1 − strength) · v + strength · centroid
```

Cette opération **n'est pas** `compute::cone_project` (qui projette
radialement vers le plus proche anchor). Elle ressemble plutôt à un
`weighted_mean` avec poids = ReLU(cos). La même variante est
réimplémentée dans :

* `dissolution/alignment.rs:75-86` (anchor weights via `cosine.max(0)`)
* `fusion/kernels/project.rs:33` (`project_rows`)

**Conclusion** : il faut **ajouter une nouvelle op** au trait
`ComputeBackend`, distincte de `cone_project` :

```rust
fn cosine_anchor_blend(
    &self,
    vector: &Tensor<f32>,
    anchors: &Tensor<f32>,
    strength: f32,
) -> Result<(Tensor<f32>, f32)>;
```

(Détails Section 5.)

### 2.4 Anti-pattern systémique : `&[f32]` au lieu de `Tensor<f32>`

L'ensemble des modules consommateurs utilise `&[f32]` ou
`&[Vec<f32>]` au lieu du type `Tensor<f32>` exposé par
`compute::tensor`. Cela provoque :

* Des conversions répétées en bord de chaque appel.
* Une perte d'information de shape (rank, dimensions).
* L'impossibilité de bénéficier des validations centralisées dans
  `compute/error.rs`.

**Corollaire architectural** : la migration introduit un **trait de
façade** par module qui consomme `&[f32]` à l'extérieur et appelle
le backend sur `Tensor<f32>` à l'intérieur. Aucun call site ne
change ; seul le corps des fonctions de façade.

---

## 3. État TO-BE — vision cible

### 3.1 Diagramme de dépendances cible

```
                              compute/
                                  ▲
                                  │ (utilisé par TOUS les modules
                                  │  ci-dessous via ComputeBackend)
                                  │
       ┌────────────────────────────────────────────────────────┐
       │                                                        │
   ┌───┴────┐  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌────┴────┐
   │ ELS    │  │ Genesis  │  │ Alignment │  │ Fusion   │  │ Consensus│
   │ (façade│  │ (façade) │  │ (façade)  │  │ (façade) │  │ (façade) │
   │  ~50LOC)  │ ~30LOC)  │  │ ~40LOC)   │  │ ~80LOC)  │  │ ~30LOC)  │
   └────────┘  └──────────┘  └───────────┘  └──────────┘  └─────────┘
```

### 3.2 Trait `ComputeBackend` étendu

Le trait passe de **18 ops à 22 ops** (4 nouvelles, voir Section 5).

### 3.3 Patron d'usage standard — 3 lignes

Avant :

```rust
use crate::els::metrics::cosine_similarity;
let cos = cosine_similarity(centroid, anchor);
```

Après :

```rust
use crate::compute::ComputeBackend;
let cos = backend.cosine_similarity_pair(&centroid_t, &anchor_t)?;
```

Le `?` est nouveau : on peut maintenant échouer proprement sur
inputs invalides au lieu de propager NaN.

### 3.4 Invariants architecturaux post-S5

| Invariant | Vérification |
|-----------|:------------:|
| **I1**. Aucune fonction publique ne calcule un dot product, une norme L2, un cosine ou un softmax sans déléguer à `ComputeBackend` | Test source-tree `s5_no_artisanal_numeric_ops` |
| **I2**. Toute fonction qui consomme un `Vec<f32>` non-fini renvoie une `Err` structurée | Property test adversarial `s5_modules_reject_nan_inputs` |
| **I3**. Les modules s'instancient avec `Arc<dyn ComputeBackend>` injecté (pattern repris de `DeviceBackend`) | Inspection du `Cargo.toml` graph + tests |
| **I4**. Les goldens de la couche numérique restent exacts ou sous tolérance documentée après refactor | Tous les goldens existants restent verts |
| **I5**. La performance ne dégrade pas de plus de 5 % sur les benchmarks Sprint 2/3 | `cargo bench --baseline pre-s5` |

---

## 4. Inventaire chirurgical des duplications

> Cette section est le **contrat de migration** : chaque ligne identifie
> un site, une cible et un test de migration. Aucun site ne doit être
> migré sans avoir une entrée ici.

### 4.1 Module `els/`

| Fichier:Lignes | Symbole | Op artisanale | Cible `ComputeBackend` | Test migration |
|----------------|---------|---------------|------------------------|----------------|
| `metrics.rs:34-40` | `dot_product` | Σ a·b | `backend.dot(a, b)` | `s5_els_metrics_dot_matches_compute` |
| `metrics.rs:46-52` | `l2_norm` | √Σ a² | `backend.l2_norm(a, -1)` | `s5_els_metrics_l2_norm_matches_compute` |
| `metrics.rs:61-69` | `l2_distance` | √Σ(a−b)² | **NEW** `backend.l2_distance(a, b)` | `s5_els_metrics_l2_distance_matches_compute` |
| `metrics.rs:79-93` | `cosine_similarity` | a·b/(‖a‖‖b‖) | **NEW** `backend.cosine_similarity_pair(a, b)` | `s5_els_metrics_cosine_matches_compute` |
| `metrics.rs:103-128` | `l2_distance_mixed`, `l2_distance_sq_mixed` | bf16 mix | conserver pour bf16 ; émule en `f32` upstream | `s5_els_metrics_mixed_unchanged` |
| `projection.rs:47-92` | `project_vector_onto_anchors` | Cosine-blend | **NEW** `backend.cosine_anchor_blend(v, a, strength)` | `s5_els_projection_matches_compute` |
| `projection.rs:94-142` | `project_with_anchor_view` | Idem (variante view) | Wrapper qui `Tensor::from_slice` puis appelle compute | idem |
| `stats.rs` (10 lignes l2_distance_sq_mixed) | snapshots stats | bf16 squared dist | inchangé (bf16 hors-scope S5) | `s5_els_stats_unchanged` |

**Action** : `els/metrics.rs` devient une **façade** mince
(~50 LOC) qui appelle le backend partagé du crate. La signature
publique `pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32`
est **préservée** mais devient :

```rust
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let backend = global_backend();   // see §5.4
    let ta = Tensor::from_slice(a, [a.len()].into()).unwrap();
    let tb = Tensor::from_slice(b, [b.len()].into()).unwrap();
    backend.cosine_similarity_pair(&ta, &tb).unwrap_or(0.0)
}
```

L'anti-pattern `unwrap` est cantonné à la façade — toutes les erreurs
sont absorbées et remappées sur la valeur géométrique de défaut
(`0.0` pour cosine, ce qui correspond au comportement actuel sur
vecteur nul).

### 4.2 Module `genesis/`

| Fichier:Lignes | Symbole | Op artisanale | Cible | Test migration |
|----------------|---------|---------------|-------|----------------|
| `meta_analyzer.rs:138-149` | `cosine_similarity` (privée) | **DUPLICAT** de `els::metrics::cosine_similarity` | Supprimer ; appeler façade `els` (ou directement le backend) | `s5_genesis_meta_no_local_cosine` |
| `meta_analyzer.rs:185-200` | softmax artisanal sur scores cosine | `((s - max)/T).exp() / Σ` | `backend.softmax(scores, -1)` après scaling par T | `s5_genesis_meta_softmax_matches_compute` |
| `dag.rs:192` | `h -= p * p.ln()` | Entropie de Shannon | **NEW** `backend.shannon_entropy(p)` | `s5_genesis_dag_entropy_matches_compute` |
| `dag.rs:264, 307` | `Σ` + `ln` (vérifications) | Trivia | inchangé (helpers ≤ 1 ligne) | n/a |
| `cardinality.rs:155-158, 186` | `h.sqrt()`, `(N).ln()` | Trivia ≤ 2 ops | inchangé | n/a |
| `router.rs:55, 200` | sommes de poids (sanity checks) | Trivia | inchangé | n/a |

**Note importante** : `genesis/meta_analyzer.rs` contient une fonction
`cosine_similarity` **strictement identique** à celle dans
`els/metrics.rs`. C'est une violation flagrante de DRY introduite
probablement parce que `meta_analyzer` ne voulait pas dépendre de
`els`. La migration utilise `compute::ComputeBackend` directement, ce
qui élimine la dépendance circulaire **et** la duplication.

### 4.3 Module `alignment/`

| Fichier:Lignes | Symbole | Op artisanale | Cible | Test migration |
|----------------|---------|---------------|-------|----------------|
| `policy.rs:256-274` | invariant I5 (anchor de-dup pairwise cosine) | Σx·y / (‖x‖‖y‖) sur N² paires | **NEW** `backend.cosine_pairwise(anchors)` (ou boucle compose `compute::dot` + `l2_norm` × N²) | `s5_alignment_policy_i5_matches_compute` |
| `compose.rs:126-150` | composition de cônes par cosine | Pour chaque a∈A, plus proche b∈B par cosine | Idem ci-dessus + `topk` pour le « plus proche » | `s5_alignment_compose_matches_compute` |
| `evolution.rs` (398 LOC) | aucune op numérique critique trouvée | n/a | inchangé | n/a |
| `breach.rs` (624 LOC) | aucune op numérique critique trouvée | n/a | inchangé | n/a |

**Décision de design** : on n'introduit PAS encore une op
`cosine_pairwise` sur `(N, D)×(M, D) → (N, M)` au S5 — cela
viendrait gonfler le trait à 23 ops. À la place, on factorise dans
un helper interne `compute::utils::pairwise_cosine` qui appelle
`matmul + l2_norm` deux fois. Si Phase 2 démontre que `cosine_pairwise`
est sur le chemin chaud, on l'élève au trait.

### 4.4 Module `consensus/`

| Fichier:Lignes | Symbole | Op artisanale | Cible | Test migration |
|----------------|---------|---------------|-------|----------------|
| `signals.rs:240-246` | `sigmoid(x)` | `1/(1+exp(-x))` | **NEW** `backend.sigmoid(t)` | `s5_consensus_signals_sigmoid_matches_compute` |
| `signals.rs:141, 171, 175` | `ln`, `sqrt`, `sqrt` | Trivia | inchangé | n/a |
| `multi_criteria.rs:361, 377, 381` | `sqrt`, `ln` (synthèses de tests) | Trivia (code de test) | inchangé | n/a |
| `scorer.rs`, `history.rs`, `verdict.rs`, `config.rs`, `detector.rs`, `multi_criteria.rs` | logique métier | n/a | inchangé | n/a |

**Cible** : on n'extrait que `sigmoid`. Le reste de `signals.rs` est
de la logique de scoring (poids, seuils) qui ne mérite pas de
migration.

### 4.5 Module `dissolution/`

| Fichier:Lignes | Symbole | Op artisanale | Cible | Test migration |
|----------------|---------|---------------|-------|----------------|
| `alignment.rs:75-86` | poids cosine + projection | Réplique de `els::projection::project_vector_onto_anchors` | Appeler `backend.cosine_anchor_blend(...)` | `s5_dissolution_alignment_matches_els_projection` |
| `alignment.rs:121` | `l2_distance` | `els::metrics::l2_distance` | Appeler `backend.l2_distance(a, b)` via façade `els` | inclus dans test ci-dessus |
| `alignment.rs:245` | sanity sum | Trivia | inchangé | n/a |
| `decoders/nearest_anchor.rs:66-74` | argmax cosine | Boucle cosine + tracking max | `backend.argmax(backend.cosine_pairwise(...))` ou variante simplifiée | `s5_dissolution_nearest_anchor_matches_compute` |
| `extractors/weighted_mean.rs` | `weighted_mean` (déjà nommé !) | À vérifier — possiblement déjà aligné | Aligner sur `backend.weighted_mean(values, weights, false)` | `s5_dissolution_extractor_matches_compute` |
| `bundle.rs:147`, `validator.rs:103` | sanity sum | Trivia | inchangé | n/a |

**Point chaud** : `dissolution/extractors/weighted_mean.rs` partage
le nom avec l'op cœur EFE `compute::weighted_mean`. Avant migration,
un test de **bisimulation** : pour les mêmes inputs, les sorties
doivent être identiques sous tolérance. Si elles divergent, c'est
un bug latent.

### 4.6 Module `fusion/kernels/` — chantier majeur

| Fichier (LOC) | Op artisanale | Cible | Test migration |
|---------------|---------------|-------|----------------|
| `softmax.rs` (243) | softmax row-wise N×N + sanity row-stochastic | `backend.softmax(scores, -1)` + assertion shape | `s5_fusion_kernels_softmax_matches_compute` |
| `attention.rs` (267) | scores Q·Kᵀ/√D + masquage + softmax sur N×N | `backend.scaled_dot_product_attention(Q, K, V, mask)` après reshape (1, 1, N, D) | `s5_fusion_kernels_attention_matches_compute` |
| `aggregate.rs` (219) | `new[i] = Σ_j w[i,j] · old[j]` | `backend.matmul(weights, old)` | `s5_fusion_kernels_aggregate_matches_compute` |
| `entropy.rs` (215) | `H_i = −Σ_j p_ij ln p_ij` row-wise + masquage | **NEW** `backend.shannon_entropy(p, -1)` + agrégation min/mean/max via `compute::stats::mean` | `s5_fusion_kernels_entropy_matches_compute` |
| `clip.rs` (202) | clipping vectoriel | inchangé (déjà trivial) | n/a |
| `mix.rs` (177) | mélange convexe `α·a + (1−α)·b` | inchangé (op affine triviale, pas de dispatch GPU intéressant à ce stade) | n/a |
| `project.rs` (212) | projection cosine-pondérée vers anchors | `backend.cosine_anchor_blend(...)` (même cible que els/projection) | `s5_fusion_kernels_project_matches_compute` |

**Stratégie** : `fusion/kernels/` est l'aspect le plus risqué car ces
kernels sont sur le chemin chaud d'un round de fusion (cf.
`M5_ARCHITECTURE.md` §10). On garde la signature publique (e.g.
`pub(crate) fn softmax(...)`) mais on remplace le corps par un
appel `ComputeBackend`. Tests de **non-régression numérique** stricts.

**Point d'attention** : les kernels `aggregate.rs` et `softmax.rs`
boucle masquage compris. Le masquage devient un argument de
`scaled_dot_product_attention` (déjà supporté) ou un pré-traitement
zero-out avant matmul. Voir §7.5 pour détail.

### 4.7 Module `collapse/`

| Fichier:Lignes | Symbole | Op artisanale | Cible | Test migration |
|----------------|---------|---------------|-------|----------------|
| `strategies/partial_respawn.rs:39` | `(threshold * grad_norm).powi(2)` | Trivia | inchangé | n/a |
| `helpers.rs:62` | `variance.sqrt()` | Trivia | inchangé | n/a |

**Action** : aucune migration substantielle. Le module utilise
déjà `els::metrics::l2_distance_sq_mixed` qui sera couvert par la
migration `els/`.

### 4.8 Synthèse — bilan AS-IS → TO-BE

| Module | LOC artisanale supprimés | LOC façade ajoutés | Δ net |
|--------|--------------------------:|--------------------:|------:|
| `els/metrics.rs` | ~150 | ~80 | −70 |
| `els/projection.rs` | ~100 | ~30 | −70 |
| `genesis/meta_analyzer.rs` | ~30 | ~10 | −20 |
| `genesis/dag.rs` | ~10 | ~5 | −5 |
| `alignment/policy.rs` | ~30 | ~15 | −15 |
| `alignment/compose.rs` | ~40 | ~15 | −25 |
| `consensus/signals.rs` | ~10 | ~5 | −5 |
| `dissolution/alignment.rs` | ~50 | ~20 | −30 |
| `dissolution/decoders/nearest_anchor.rs` | ~20 | ~10 | −10 |
| `fusion/kernels/softmax.rs` | ~150 | ~30 | −120 |
| `fusion/kernels/attention.rs` | ~200 | ~50 | −150 |
| `fusion/kernels/aggregate.rs` | ~100 | ~30 | −70 |
| `fusion/kernels/entropy.rs` | ~150 | ~30 | −120 |
| `fusion/kernels/project.rs` | ~120 | ~30 | −90 |
| **TOTAL** | **~1 160** | **~360** | **−800** |

---

## 5. Extensions requises au trait `ComputeBackend`

### 5.1 Opérations actuelles (Sprint 1-3) — rappel

```rust
// 18 ops déjà présentes :
fn dot, fn l2_norm, fn l2_normalize, fn mean, fn argmax,
fn matmul, fn matmul_fused_gelu, fn softmax, fn log_softmax,
fn gelu, fn layer_norm,
fn cone_project, fn weighted_mean, fn moe_reconcile,
fn scaled_dot_product_attention, fn topk,
fn multinomial_sample, fn kl_divergence, fn std_dev
```

### 5.2 Ops à ajouter en S5

| # | Signature | Sémantique | Justification |
|---|-----------|------------|---------------|
| 19 | `fn l2_distance(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>>` | `‖a − b‖₂` rank-1 ou batché rank-2 | `els::metrics::l2_distance`, `dissolution::alignment` |
| 20 | `fn cosine_similarity_pair(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<f32>` | `a·b / (‖a‖‖b‖)`, retourne 0 si dénominateur < ε | `els::metrics`, `genesis::meta_analyzer`, `alignment::policy/compose`, `dissolution` |
| 21 | `fn cosine_anchor_blend(&self, vector: &Tensor<f32>, anchors: &Tensor<f32>, strength: f32) -> Result<(Tensor<f32>, f32)>` | Centroïde pondéré par cos⁺ + blend convexe avec `vector`, retourne (résultat, delta L2) | `els::projection`, `dissolution::alignment`, `fusion::kernels::project` |
| 22 | `fn shannon_entropy(&self, p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>` | `−Σ_i p_i · ln(max(p_i, ε))` row-wise | `fusion::kernels::entropy`, `genesis::dag` |
| 23 | `fn sigmoid(&self, x: &Tensor<f32>) -> Result<Tensor<f32>>` | `1 / (1 + exp(−x))`, IEEE-754 stable | `consensus::signals::sigmoid` |

**Total** : 5 nouvelles ops → trait passe de **18 → 23 ops**.

### 5.3 Spécifications détaillées

#### 5.3.1 `l2_distance`

```rust
fn l2_distance(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>>;
```

* **Inputs** : `a, b` de même shape, rank ∈ {1, 2}.
* **Output** :
  * Rank-1 → scalaire (`Tensor` rank-0).
  * Rank-2 `(B, D)` → `(B,)` distances par ligne.
* **Erreurs** :
  * `ShapeMismatch` si rank ≠ {1, 2} ou shapes ≠.
  * `NumericalInstability` si non-finie (NaN dans inputs).
* **Implémentation CPU** : `(backend.dot(a−b, a−b))^0.5` ou inline
  Kahan pour précision.
* **Tolérance cross-backend** : `rtol = 1e-5`.

#### 5.3.2 `cosine_similarity_pair`

```rust
fn cosine_similarity_pair(
    &self, a: &Tensor<f32>, b: &Tensor<f32>,
) -> Result<f32>;
```

* **Inputs** : deux vecteurs rank-1, même longueur.
* **Output** : scalaire `f32` ∈ `[−1, 1]`.
* **Sémantique du dénominateur nul** : si `‖a‖·‖b‖ < ε` (avec
  `ε = f32::EPSILON`), retourne `Ok(0.0)` (limite géométrique
  d'orientations indéfinies).
* **Différence vs `els::metrics::cosine_similarity` actuel** : ne
  panique pas, retourne `Result`. Comportement géométrique identique.
* **Variante batchée future (S6+)** : `cosine_similarity_batch(A: (N, D), B: (M, D)) -> (N, M)` — non incluse en S5.

#### 5.3.3 `cosine_anchor_blend`

```rust
fn cosine_anchor_blend(
    &self,
    vector: &Tensor<f32>,
    anchors: &Tensor<f32>,
    strength: f32,
) -> Result<(Tensor<f32>, f32)>;
```

* **Inputs** :
  * `vector : (D,)`,
  * `anchors : (N, D)` avec `N ≥ 1`,
  * `strength ∈ [0, 1]`.
* **Algorithme** (équivalent strict à `els::projection::project_vector_onto_anchors`) :
  ```text
  weights[i] = max(0, cosine(vector, anchors[i]))   for i in 0..N
  Σw = Σ weights
  if Σw < ε:
      return (vector, 0.0)        # identity, no delta
  w_norm[i] = weights[i] / Σw
  centroid = Σ_i w_norm[i] · anchors[i]
  result   = (1 − strength) · vector + strength · centroid
  delta    = ‖vector − result‖₂
  return (result, delta)
  ```
* **Output** : tuple `(result_tensor : (D,), delta : f32)`.
* **Erreurs** :
  * `ShapeMismatch` sur dimensions incompatibles.
  * `NumericalInstability` si `strength` non-fini ou hors `[0, 1]`,
    ou si tous les anchors contiennent NaN.
* **Tolérance cross-backend** : `rtol = 1e-4` (compose softmax-like
  + matmul).

#### 5.3.4 `shannon_entropy`

```rust
fn shannon_entropy(&self, p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;
```

* **Inputs** : `p` rank ≥ 1, axis valide.
* **Sémantique** : `H = −Σ_i p_i · ln(max(p_i, ε))` avec
  `ε = 1e-12` pour éviter `ln(0)`.
* **Validation** :
  * Rejette `p_i < 0` ou non-fini (cohérent avec `kl_divergence`
    durci à l'audit 5).
  * Ne **normalise pas** `p` — l'appelant doit fournir une
    distribution. (Si on normalise, le contrat devient confus pour
    les usages d'entropie sur scores non-normalisés.)
* **Output** : tenseur de rank `rank(p) − 1` (axis réduit).
* **Tolérance** : `rtol = 1e-5`.

#### 5.3.5 `sigmoid`

```rust
fn sigmoid(&self, x: &Tensor<f32>) -> Result<Tensor<f32>>;
```

* **Sémantique** : `σ(x) = 1 / (1 + exp(−x))`, élément par élément.
* **Stabilité numérique** :
  * Pour `x > 0` : `1 / (1 + exp(−x))` direct.
  * Pour `x < 0` : `exp(x) / (1 + exp(x))` — évite l'overflow de
    `exp(−x)` quand `x` très négatif.
* **Output** : même shape que l'input.
* **Tolérance** : `rtol = 1e-6` (pure transcendantale).

### 5.4 Pattern d'injection du backend

Le trait `ComputeBackend` est déjà un `dyn`-friendly trait
(`Send + Sync`). On ajoute un singleton statique pour les façades
**legacy** qui ne peuvent pas être pénétrées par injection sans
casser leur API :

```rust
// crates/efe-core/src/compute/global.rs (NEW)

use std::sync::OnceLock;
use std::sync::Arc;
use crate::compute::{ComputeBackend, CpuBackend};

static GLOBAL: OnceLock<Arc<dyn ComputeBackend>> = OnceLock::new();

/// Returns the process-wide default backend. Initialised on first
/// access to `CpuBackend::new()`. Tests may override via `set_global`
/// (test-only).
pub fn global_backend() -> Arc<dyn ComputeBackend> {
    GLOBAL.get_or_init(|| Arc::new(CpuBackend::new())).clone()
}

#[cfg(test)]
pub fn set_global_for_test(b: Arc<dyn ComputeBackend>) {
    let _ = GLOBAL.set(b);
}
```

**Justification** : les façades comme `els::metrics::cosine_similarity`
ont une signature `fn(a, b) -> f32` qu'on **doit** préserver pour ne
pas casser des dizaines de call sites. Le singleton est le seul moyen
d'injecter un backend sans changer leur signature.

Pour les modules nouvellement migrés (e.g. `fusion/`), on **préfère**
l'injection explicite via `Arc<dyn ComputeBackend>` dans le constructeur,
suivant le pattern existant de `DeviceBackend` (cf. `fusion/mvp_cpu.rs:39`).

---

## 6. Matrice de dépendances inter-modules

### 6.1 Ordre topologique de migration (déduit du graphe d'imports)

```
Phase A1 (préalable)  : Étendre ComputeBackend (5 nouvelles ops)
                              │
                              ▼
Phase A2  : els/metrics                       ◄── racine de la dépendance
                              │
                              ▼
Phase A3  : els/projection                    ◄── consomme els/metrics
                              │
                              ▼
Phase A4  : genesis/{dag, meta_analyzer}      ◄── indépendant des autres
                              │
                              ▼
Phase A5  : alignment/{policy, compose}       ◄── indépendant des autres
                              │
                              ▼
Phase A6  : consensus/signals (sigmoid)       ◄── indépendant
                              │
                              ▼
Phase A7  : dissolution/{alignment, decoders, extractors}  ◄── consomme els
                              │
                              ▼
Phase A8  : fusion/kernels/{softmax, attention, aggregate, entropy, project}
                              │  ◄── chantier le plus lourd, dépend du backend complet
                              ▼
Phase A9  : audit cross-module + goldens cross-module
```

### 6.2 Matrice de dépendances post-migration

|             | compute | els | genesis | alignment | consensus | dissolution | fusion | collapse |
|-------------|:-------:|:---:|:-------:|:---------:|:---------:|:-----------:|:------:|:--------:|
| **compute** |    —    |     |         |           |           |             |        |          |
| **els**     |    ●    |  —  |         |           |           |             |        |          |
| **genesis** |    ●    |     |    —    |           |           |             |        |          |
| **alignment**|   ●    |     |         |     —     |           |             |        |          |
| **consensus**|   ●    |     |         |           |     —     |             |        |          |
| **dissolution**|  ●  |  ●  |         |           |           |      —      |        |          |
| **fusion**   |   ●    |     |         |           |           |             |   —    |          |
| **collapse** |        |  ●  |         |           |           |             |        |    —     |

`●` = dépend directement. La matrice est triangulaire inférieure
(pas de cycles), confirmée par `cargo check`.

### 6.3 Risque de cycle introduit

**Aucun.** `ComputeBackend` est défini dans `efe-core::compute`,
qui est en racine du DAG (ne dépend que de `compute::tensor`,
`compute::error`, `compute::shape`). Tous les autres modules
peuvent l'importer sans risque de cycle.

---

## 7. Plan de migration en 9 phases (A1–A9)

> Chaque phase suit le pattern audit-friendly :
> **(1) Implémenter** → **(2) Tester** → **(3) Auditer** → **(4) Documenter**.
> Aucune phase n'est mergée tant que sa suite de tests n'est pas verte.

### 7.1 Phase A1 — Étendre `ComputeBackend` (1 jour)

**Fichiers touchés** :
* `crates/efe-core/src/compute/backend.rs` (+5 signatures)
* `crates/efe-core/src/compute/cpu/mod.rs` (+5 délégations)
* `crates/efe-core/src/compute/cpu/{linalg.rs, activ.rs, stats.rs, consensus.rs}` (+5 implémentations)
* `crates/efe-core/src/compute/cuda/mod.rs` (+5 stubs `Err(NotImplemented)`)
* `crates/efe-core/src/compute/global.rs` (NOUVEAU — singleton)
* `crates/efe-core/src/compute/mod.rs` (+ pub use)

**Tests à ajouter** :
* `cpu/linalg.rs` : `l2_distance_basic`, `l2_distance_zero_vector`,
  `l2_distance_rejects_shape_mismatch`, `l2_distance_rejects_nan`.
* `cpu/consensus.rs` : `cosine_similarity_pair_identity`,
  `_orthogonal`, `_zero_norm`, `_rejects_nan`.
* `cpu/consensus.rs` : `cosine_anchor_blend_strength_zero_returns_input`,
  `_strength_one_moves_to_centroid`, `_orthogonal_anchors_identity`,
  `_rejects_strength_out_of_range`.
* `cpu/sampling.rs` : `shannon_entropy_uniform_max`, `_concentrated_zero`,
  `_rejects_negative`, `_self_consistency` (`H + KL == log N` pour
  uniforme).
* `cpu/activ.rs` : `sigmoid_zero_is_half`, `_monotone`,
  `_bounded_unit_range`, `_stable_large_negative`.

**Goldens à ajouter** : 5 fichiers JSON dans `tests/golden/` :
* `l2_distance/basic.json`
* `cosine_similarity/orthogonal.json`
* `cosine_anchor_blend/half_strength.json`
* `shannon_entropy/uniform.json`
* `sigmoid/reference_points.json`

**Critères de sortie A1** :
* [ ] 23/23 ops implémentées en `CpuBackend` (vs 18 avant)
* [ ] 5 nouveaux goldens chargés via le harness Sprint 5 golden
* [ ] +20 tests unitaires verts
* [ ] Clippy strict pur Rust ET `--features accelerate` sans warning

### 7.2 Phase A2 — `els/metrics.rs` (1 jour)

**Fichiers touchés** :
* `crates/efe-core/src/els/metrics.rs` (corps réécrit)
* `crates/efe-core/src/els/mod.rs` (re-exports inchangés)

**Stratégie** :
1. Ajouter en haut du fichier `use crate::compute::global::global_backend;`
2. Réécrire `dot_product`, `l2_norm`, `l2_distance`, `cosine_similarity`
   pour appeler le backend.
3. Conserver les fonctions `*_mixed` (bf16) **inchangées** — bf16
   n'est pas dans le scope S5 du backend.
4. Conserver les `assert_eq!` panics sur length mismatch — c'est le
   contrat documenté du module ; le backend renverrait `ShapeMismatch`
   mais on l'absorbe en panic pour préserver l'API.

**Tests à ajouter** :
* `s5_els_metrics_dot_matches_compute` — 1 000 inputs aléatoires
  comparés à `backend.dot(a, b)`.
* Idem pour `l2_norm`, `l2_distance`, `cosine_similarity`.
* Tests de régression : tous les tests existants
  `els::metrics::tests::*` restent verts sans modification.

**Critères de sortie A2** :
* [ ] 4 fonctions migrées vers `ComputeBackend`
* [ ] 100 % des tests existants `els::metrics` toujours verts
* [ ] 4 nouveaux tests de bisimulation cross-impl

### 7.3 Phase A3 — `els/projection.rs` (1 jour)

**Fichiers touchés** :
* `crates/efe-core/src/els/projection.rs` (corps réécrit)

**Stratégie** :
1. Remplacer les deux fonctions `project_vector_onto_anchors` et
   `project_with_anchor_view` par des appels à
   `backend.cosine_anchor_blend(vector, anchors_tensor, strength)`.
2. Préserver leur signature `Result<(Vec<f32>, f32)>`.
3. La conversion `&[Vec<f32>] → Tensor<f32>(N, D)` se fait dans le
   corps de la fonction (allocation d'un buffer plat).

**Tests à ajouter** :
* `s5_els_projection_matches_compute_cosine_blend` — 1 000 inputs
  aléatoires (anchors variés, strengths variés).
* Tests existants `els::projection::tests::*` restent verts.

**Critères de sortie A3** :
* [ ] 2 fonctions migrées, signature préservée
* [ ] +1 test de bisimulation
* [ ] tests existants ≥ 9 toujours verts

### 7.4 Phase A4 — `genesis/{dag, meta_analyzer}` (1 jour)

**Fichiers touchés** :
* `crates/efe-core/src/genesis/meta_analyzer.rs` (suppression du
  duplicat + softmax via backend)
* `crates/efe-core/src/genesis/dag.rs` (entropie via backend)

**Étape 1 — `meta_analyzer.rs`** :
1. Supprimer la fonction privée `cosine_similarity` (lignes 138-149).
2. Importer `els::metrics::cosine_similarity` (qui utilise déjà
   le backend après A2).
3. Remplacer le softmax artisanal lignes 185-200 par :
   ```rust
   let mut scaled = scores.clone();
   for s in &mut scaled { *s /= self.temperature; }
   let scaled_t = Tensor::from_slice(&scaled, [scores.len()].into())?;
   let probs_t = self.backend.softmax(&scaled_t, -1)?;
   let probs: Vec<f32> = probs_t.as_slice()?.to_vec();
   ```

**Étape 2 — `dag.rs`** :
1. Ligne 192 : remplacer la boucle `h -= p * p.ln()` par :
   ```rust
   let p_t = Tensor::from_slice(&probs, [probs.len()].into())?;
   let h = self.backend.shannon_entropy(&p_t, -1)?.as_slice()?[0];
   ```

**Tests à ajouter** :
* `s5_genesis_meta_no_local_cosine` — vérifie via `grep` source
  que `fn cosine_similarity` n'apparaît plus dans `meta_analyzer.rs`.
* `s5_genesis_meta_softmax_matches_compute`
* `s5_genesis_dag_entropy_matches_compute`

**Critères de sortie A4** :
* [ ] Duplicat supprimé (vérifié par test source)
* [ ] Softmax + entropie via backend
* [ ] +3 tests de bisimulation

### 7.5 Phase A5 — `alignment/{policy, compose}` (1.5 jours)

**Fichiers touchés** :
* `crates/efe-core/src/alignment/policy.rs` (invariant I5)
* `crates/efe-core/src/alignment/compose.rs` (composition de cônes)

**Stratégie** :
1. Helper interne dans `crates/efe-core/src/compute/utils.rs` :
   ```rust
   pub fn pairwise_cosine(
       backend: &dyn ComputeBackend,
       anchors: &[&[f32]],
   ) -> Result<Vec<Vec<f32>>>;
   ```
   Implémentation : pour chaque (i, j), calculer
   `backend.cosine_similarity_pair(anchors[i], anchors[j])`.
2. `policy.rs:267-278` (invariant I5) appelle ce helper.
3. `compose.rs:130-150` appelle ce helper et utilise `backend.argmax`
   (déjà existant) pour trouver le « plus proche b ∈ B ».

**Tests à ajouter** :
* `s5_alignment_policy_i5_matches_compute` — bisimulation sur
  500 jeux d'anchors aléatoires.
* `s5_alignment_compose_matches_compute` — bisimulation sur
  500 cas.

**Critères de sortie A5** :
* [ ] 2 fichiers migrés
* [ ] +2 tests de bisimulation
* [ ] Tous les tests existants `alignment::*` toujours verts (≥ 50 tests)

### 7.6 Phase A6 — `consensus/signals.rs` (0.5 jour)

**Fichiers touchés** :
* `crates/efe-core/src/consensus/signals.rs` (suppression de
  `fn sigmoid` + appels)

**Stratégie** :
1. Supprimer la fonction privée `sigmoid` (lignes 240-246).
2. Remplacer les call sites par `backend.sigmoid(x_tensor)`.

**Tests à ajouter** :
* `s5_consensus_signals_sigmoid_matches_compute`
* Tous les tests existants `consensus::signals::tests::*` restent verts.

**Critères de sortie A6** :
* [ ] Sigmoid supprimé du module
* [ ] +1 test de bisimulation

### 7.7 Phase A7 — `dissolution/` (1 jour)

**Fichiers touchés** :
* `crates/efe-core/src/dissolution/alignment.rs`
* `crates/efe-core/src/dissolution/decoders/nearest_anchor.rs`
* `crates/efe-core/src/dissolution/extractors/weighted_mean.rs`

**Stratégie** :
1. `alignment.rs:75-86` : remplacer le calcul anchor weights +
   blending par un appel à `backend.cosine_anchor_blend` (si compatible)
   ou par une suite `backend.cosine_similarity_pair` + accumulation.
2. `alignment.rs:121` : `l2_distance` via façade `els`.
3. `decoders/nearest_anchor.rs:66` : remplacer la boucle
   `cosine_similarity(projected, anchor)` + tracking max par :
   ```rust
   let scores = anchors.iter()
       .map(|a| backend.cosine_similarity_pair(&proj_t, &a_t))
       .collect::<Result<Vec<_>>>()?;
   let scores_t = Tensor::from_slice(&scores, [scores.len()].into())?;
   let idx = backend.argmax(&scores_t, -1)?.as_slice()?[0];
   ```
4. `extractors/weighted_mean.rs` : auditer le code actuel,
   si l'algorithme correspond à `compute::weighted_mean`, le
   remplacer ; sinon documenter la différence.

**Tests à ajouter** :
* `s5_dissolution_alignment_matches_els_projection`
* `s5_dissolution_nearest_anchor_matches_compute`
* `s5_dissolution_extractor_matches_compute`

**Critères de sortie A7** :
* [ ] 3 fichiers migrés
* [ ] +3 tests de bisimulation
* [ ] Tests E2E `dissolution_e2e.rs` toujours verts

### 7.8 Phase A8 — `fusion/kernels/` (3 jours, le gros chantier)

**Fichiers touchés** :
* `crates/efe-core/src/fusion/kernels/softmax.rs`
* `crates/efe-core/src/fusion/kernels/attention.rs`
* `crates/efe-core/src/fusion/kernels/aggregate.rs`
* `crates/efe-core/src/fusion/kernels/entropy.rs`
* `crates/efe-core/src/fusion/kernels/project.rs`

**A8.1 — `softmax.rs` (0.5 jour)** :
* Remplacer la boucle row-wise par `backend.softmax(scores_t, -1)`.
* Conserver le post-traitement de masquage (zero-out des positions
  inactives).

**A8.2 — `attention.rs` (1 jour)** :
* Construire `Q, K, V` au format rank-4 `(1, 1, N, D)` pour matcher
  la signature `scaled_dot_product_attention`.
* Le masque additif `(N, N)` reste tel quel (déjà supporté par
  l'op compute).
* Précaution : le scaling `1/(√D · temperature)` actuel inclut
  une `temperature`. Comme `compute::SDPA` fait `1/√D` seulement,
  il faut **diviser Q par `temperature`** avant l'appel pour
  obtenir `1/(√D · temperature)`. Tests strictes pour s'en assurer.

**A8.3 — `aggregate.rs` (0.5 jour)** :
* `aggregate(old, new, mask, softmax_weights, ...)` est en réalité
  un `matmul(softmax_weights, old)` où `softmax_weights` est `(N, N)`
  et `old` est `(N, D)`.
* Utiliser `backend.matmul(&weights_t, &old_t)?` puis appliquer le
  masque actif lignes/colonnes.

**A8.4 — `entropy.rs` (0.5 jour)** :
* Remplacer la boucle `H_i = −Σ_j p_ij · ln(p_ij)` par
  `backend.shannon_entropy(p_t, -1)`.
* L'agrégation min/mean/max ensuite via `compute::stats::min/mean/max`.

**A8.5 — `project.rs` (0.5 jour)** :
* Boucle d'application de `cosine_anchor_blend` row-wise.
* Possibilité d'introduire une variante batched dans S6+ ; en S5,
  on fait `N` appels indépendants.

**Tests à ajouter** : 5 tests de bisimulation, un par kernel, plus
les tests E2E `fusion::*` qui doivent rester verts.

**Critères de sortie A8** :
* [ ] 5 kernels migrés
* [ ] +5 tests de bisimulation
* [ ] Tests E2E fusion verts (`tests/fusion_e2e.rs` ≥ 12 tests)
* [ ] Pas de régression performance > 10 % (tolérance plus large
  que les 5 % globaux car ces kernels sont sur le chemin chaud
  et la bisimulation passe par un plus grand nombre d'allocations).

### 7.9 Phase A9 — Audit cross-module + goldens cross-module (1 jour)

**Fichiers touchés** :
* `crates/efe-core/tests/compute_sprint5_cross_module.rs` (NOUVEAU)
* `crates/efe-core/tests/golden/cross_module/*.json` (NOUVEAU, 6 goldens)
* `docs/compute/tolerances.md` (section cross-module)
* `crates/efe-core/src/compute/ROADMAP.md` (Phase 1 cochée)

**Goldens cross-module à produire** :
1. `cross_module/dissolution_full_pipeline.json` — Π(latents) →
   weighted_mean → cosine_anchor_blend → nearest_anchor.
2. `cross_module/genesis_meta_to_dag.json` — meta_analyzer
   `softmax → entropy(probs)` doit donner exactement la valeur
   indiquée.
3. `cross_module/alignment_compose_two_cones.json` — compose
   d'un cône A (3 anchors) avec un cône B (4 anchors).
4. `cross_module/fusion_one_round.json` — un round complet
   `softmax(scores) → aggregate → project_rows → clip`.
5. `cross_module/els_projection_then_distance.json` —
   `cosine_anchor_blend(v, A) → l2_distance(out, anchor[0])`.
6. `cross_module/consensus_signals_full_score.json` — un
   scoring complet utilisant sigmoid.

**Test source-tree** :
```rust
// tests/s5_no_artisanal_numeric_ops.rs
fn no_softmax_outside_compute() {
    let blacklist = ["softmax", "scaled_dot_product_attention"];
    let modules = ["alignment", "consensus", "dissolution",
                    "fusion/mvp_cpu", "fusion/replay",
                    "genesis", "els", "collapse"];
    for module in modules {
        for path in glob(&format!("crates/efe-core/src/{}/**/*.rs", module)) {
            let src = read_to_string(&path).unwrap();
            // Allow comments and string literals; only fail on
            // bare identifier matches.
            for kw in &blacklist {
                if has_bare_identifier(&src, kw) {
                    panic!("Module {} contains artisanal `{}` at {:?}",
                           module, kw, path);
                }
            }
        }
    }
}
```

**Critères de sortie A9** :
* [ ] 6 goldens cross-module verts
* [ ] Test source-tree confirmant 0 op artisanale
* [ ] `tolerances.md` étendu
* [ ] ROADMAP.md : Phase 1 marquée comme livrée

---

## 8. Stratégie de test et goldens cross-module

### 8.1 Trois niveaux de tests pour la migration

| Niveau | Granularité | Exemple | Volume cible |
|--------|-------------|---------|:------------:|
| **L1** Unit | Op `ComputeBackend` isolée | `cpu/consensus.rs::cosine_anchor_blend_strength_zero_returns_input` | +20 tests |
| **L2** Bisimulation | Module appelant ↔ backend | `s5_els_metrics_dot_matches_compute` | +18 tests |
| **L3** Cross-module | Pipeline multi-module | `s5_cross_dissolution_pipeline_matches_golden` | +6 tests |

**Total tests S5 : ~+44**, portant le total à `1 276 + 44 = 1 320`
(côté pur Rust ; +56 avec accelerate).

### 8.2 Pattern de test de bisimulation

```rust
#[test]
fn s5_els_metrics_l2_norm_matches_compute() {
    let backend = CpuBackend::new();
    let mut rng = StdRng::seed_from_u64(42);
    for _ in 0..500 {
        let n = rng.gen_range(2..=64);
        let v: Vec<f32> = (0..n).map(|_| rng.gen_range(-10.0..10.0)).collect();
        // Implementation 1 : module-level façade.
        let got_facade = els::metrics::l2_norm(&v);
        // Implementation 2 : direct backend call.
        let t = Tensor::<f32>::from_slice(&v, [v.len()].into()).unwrap();
        let got_backend = backend.l2_norm(&t, -1).unwrap()
            .as_slice().unwrap()[0];
        assert!(
            (got_facade - got_backend).abs() < 1e-5,
            "façade {} ≠ backend {} for v={:?}",
            got_facade, got_backend, v,
        );
    }
}
```

### 8.3 Pattern de golden cross-module

```json
{
  "op": "cross_module/dissolution_full_pipeline",
  "version": "1.0",
  "note": "Pipeline E2E : projection cosine_anchor_blend (s = 0.7) puis nearest_anchor sur 3 anchors. Vérifie que le couplage els::projection ↔ dissolution::nearest_anchor produit bit-pour-bit la même chose qu'un appel manuel orchestré.",
  "inputs": {
    "centroid": { "shape": [4], "data": [0.1, 0.2, 0.3, 0.4] },
    "anchors":  { "shape": [3, 4], "data": [...] },
    "strength": 0.7
  },
  "expected": {
    "projected": { "shape": [4], "data": [...] },
    "delta": 0.123,
    "nearest_anchor_idx": 1,
    "nearest_anchor_score": 0.987
  },
  "tolerance": { "rtol": 1.0e-4, "atol": 1.0e-5 }
}
```

### 8.4 Test source-tree — détection automatique de régression

Détaillé en §7.9. Le test fail si quelqu'un réintroduit une op
artisanale après le refactor. Acts as a **regression latch**
permanent.

---

## 9. Contrats de tolérance unifiés

### 9.1 Hiérarchie des tolérances

```
                  ┌────────────────────────────────────┐
                  │   Tolérance "atomique"             │
                  │   1 op compute pure                │
                  │   rtol = 1e-5, atol = 1e-7         │
                  └────────────────────────────────────┘
                                    │ compose
                                    ▼
                  ┌────────────────────────────────────┐
                  │   Tolérance "BLAS"                 │
                  │   1 op compute + dispatch BLAS     │
                  │   rtol = 1e-4, atol = 1e-5         │
                  └────────────────────────────────────┘
                                    │ compose
                                    ▼
                  ┌────────────────────────────────────┐
                  │   Tolérance "cross-module"         │
                  │   N ops compute en pipeline        │
                  │   rtol = 2e-4, atol = 5e-5         │
                  └────────────────────────────────────┘
                                    │ compose
                                    ▼
                  ┌────────────────────────────────────┐
                  │   Tolérance "fusion round complet" │
                  │   ~10 ops chainées                 │
                  │   rtol = 5e-4, atol = 1e-4         │
                  └────────────────────────────────────┘
```

### 9.2 Tableau de tolérance par op nouvelle

| Op | rtol | atol | Notes |
|----|:----:|:----:|-------|
| `l2_distance` | `1e-5` | `1e-7` | Pure : sub + sq + sum + sqrt |
| `cosine_similarity_pair` | `1e-5` | `1e-7` | Pure : dot + 2 norms + divide |
| `cosine_anchor_blend` | `1e-4` | `1e-5` | Compose : N×cosine + softmax-like + matmul |
| `shannon_entropy` | `1e-5` | `1e-6` | Pure : 1 sum, transcendantale `ln` |
| `sigmoid` | `1e-6` | `1e-7` | Pure transcendantale |

### 9.3 Mise à jour de `docs/compute/tolerances.md`

Ajout d'une **Section 5 : Cross-module tolerances**, avec un tableau
décrivant chaque chaîne migrée et sa tolérance attendue.

---

## 10. Critères d'acceptation par phase

### 10.1 Tableau récapitulatif

| Phase | Tests +Δ | Goldens +Δ | LOC −Δ | Durée | Prérequis |
|-------|:--------:|:----------:|:------:|:-----:|-----------|
| A1 | +20 | +5 | 0 | 1 j | aucune |
| A2 | +4 | 0 | −70 | 1 j | A1 |
| A3 | +1 | 0 | −70 | 1 j | A2 |
| A4 | +3 | 0 | −25 | 1 j | A1 (indép. de A2) |
| A5 | +2 | 0 | −40 | 1.5 j | A1 |
| A6 | +1 | 0 | −5 | 0.5 j | A1 |
| A7 | +3 | 0 | −40 | 1 j | A2, A3 |
| A8 | +5 | 0 | −550 | 3 j | A1, A4 |
| A9 | +5 (source-tree, parse goldens) | +6 | 0 | 1 j | A2-A8 |
| **Total** | **+44** | **+11** | **−800** | **11 j** | — |

### 10.2 Critères globaux Sprint 5

Reprise de §1.3 — récapitulé pour clôture :

* [ ] C1 : 100 % ops via `ComputeBackend`
* [ ] C2 : 0 implémentation artisanale détectée
* [ ] C3 : 100 % NaN → `Err`
* [ ] C4 : ≥ 1 320 tests verts
* [ ] C5 : 12 tests cross-module
* [ ] C6 : 6 goldens cross-module
* [ ] C7 : `tolerances.md` étendu
* [ ] C8 : régression perf < 5 %
* [ ] C9 : clippy strict 2 configs
* [ ] C10 : ROADMAP Phase 1 cochée

---

## 11. Plan jour-par-jour Sprint 5

### Semaine 1

| Jour | Phase | Livrable |
|------|-------|----------|
| J1 | A1 | 5 nouvelles ops `ComputeBackend` + 20 tests + 5 goldens |
| J2 | A2 | `els/metrics.rs` migré |
| J3 | A3 | `els/projection.rs` migré |
| J4 | A4 | `genesis/{dag, meta_analyzer}` migrés |
| J5 | A5 | `alignment/{policy, compose}` migrés |

### Semaine 2

| Jour | Phase | Livrable |
|------|-------|----------|
| J6 | A6 | `consensus/signals` migré (matin) + démarrage A7 (après-midi) |
| J7 | A7 | `dissolution/` migré |
| J8 | A8.1-A8.2 | `fusion/kernels/{softmax, attention}` migrés |
| J9 | A8.3-A8.5 | `fusion/kernels/{aggregate, entropy, project}` migrés |
| J10 | A9 | Audit cross-module + 6 goldens + 12 tests cross-module + doc |
| J11 | A9 | Tampons : finitions, audit final type 5e passe |

### Buffer

Le plan inclut **0.5 jour de buffer** sur 11 jours, soit ~5 %
contingence. Si A8 dépasse, le buffer est consommé. Si A8 finit
en avance, on entame **Phase 2 CUDA préparatoire** (premières
signatures `CudaBackend`).

---

## 12. Risques et mitigations

| # | Risque | Probabilité | Impact | Mitigation |
|---|--------|:-----------:|:------:|------------|
| R1 | Régression numérique sous-tolérance | Moyenne | Élevé | Goldens existants restent oracles ; bisimulation 500 cas par migration |
| R2 | Régression performance > 5 % sur fusion | Moyenne | Moyen | Bench A/B avant/après chaque migration A8 ; rollback granulaire si > 10 % |
| R3 | API publique `els::metrics::*` cassée | Faible | Élevé | Façade conserve la signature ; tests downstream (consommateurs) restent verts |
| R4 | Cycle d'imports introduit | Très faible | Élevé | `cargo check` après chaque commit ; le DAG actuel est déjà respecté |
| R5 | Fusion kernels comportement subtil non capturé par tests | Moyenne | Élevé | Tests E2E `fusion_e2e.rs` complets avant/après ; revue de code par 2e relecteur sur A8 |
| R6 | `cosine_anchor_blend` ne couvre pas une variante exotique | Faible | Moyen | Audit complet des 3 sites consommateurs avant figeage de la signature ; possibilité d'ajouter un paramètre `weight_kernel: WeightKernel::Positive | Squared` |
| R7 | Backend global singleton cause des problèmes dans tests parallèles | Moyenne | Faible | `OnceLock` est thread-safe ; `set_global_for_test` réservé à des tests `serial_test` |
| R8 | bf16 paths divergent | Très faible | Moyen | bf16 hors scope S5 (documenté en §4.1) ; les façades `*_mixed` restent inchangées |

---

## 13. Rollback et compatibilité ascendante

### 13.1 Granularité du rollback

Chaque phase Ai est mergée comme **une PR distincte** avec son propre
test gate. Si une phase casse en CI :
* La PR est rejetée et le code reste en état Ai-1.
* Les phases ultérieures (Aj, j > i) ne sont pas bloquées **ssi**
  elles ne dépendent pas de Ai (cf. §6.1 — A4, A5, A6 sont
  indépendantes).

### 13.2 Compatibilité publique

* **Trait `ComputeBackend`** : seules des **additions** (+5 ops). Aucune
  signature existante modifiée. Aucun call site externe cassé.
* **Modules consommateurs** (`els/metrics`, `genesis`, etc.) : leur
  signature publique est **strictement préservée**. Le corps change.
* **Re-exports** dans `lib.rs` : inchangés.

### 13.3 Versioning sémver

Le crate `efe-core` est en `0.0.1`. Toute la migration tient en
**ajouts** + **internes**. Aucun bump majeur requis. Sortie en
`0.0.2` après S5.

---

## 14. Préparation Phase 2 CUDA

Le S5 est un **investissement** dont la rentabilité se matérialise en
Phase 2.

### 14.1 Effet de levier

Avant S5 :
* Pour porter `fusion::kernels::softmax` à CUDA, il faut : porter le
  kernel `softmax` CUDA + adapter chaque consommateur pour qu'il
  utilise le nouveau path GPU + tester chaque chaîne.
* Multiplier par 5 kernels artisanaux × 7 consommateurs = ~35 sites
  à toucher.

Après S5 :
* Pour porter `fusion::kernels::softmax` à CUDA, il faut : porter
  `compute::cpu::activ::softmax` à `compute::cuda::activ::softmax`,
  un seul site.
* L'ensemble des 7 consommateurs bascule automatiquement quand
  l'instance `Arc<dyn ComputeBackend>` passe de `CpuBackend` à
  `CudaBackend`.

### 14.2 Liste des kernels CUDA Phase 2 (mise à jour S5)

Avec les 5 nouvelles ops S5, la Phase 2 vise **23 kernels CUDA**
au lieu de 18. Mais 4 des 5 nouvelles ops (`l2_distance`,
`cosine_similarity_pair`, `cosine_anchor_blend`, `shannon_entropy`)
sont des **compositions de kernels existants** (matmul, softmax,
log_softmax, l2_norm). Leur portage CUDA réutilise les kernels du
plan original. Seul **`sigmoid`** est un kernel élémentaire nouveau,
trivial (~50 LOC CUDA).

### 14.3 Goldens cross-module — réutilisation Phase 2

Les 6 goldens cross-module produits en A9 deviennent les **oracles
Phase 2** : pour valider que `CudaBackend` fonctionne, on rejoue
exactement ces 6 pipelines et on vérifie sortie sous tolérance
documentée. C'est l'auto-validation Phase 2 préparée gratuitement.

---

## 15. Annexes

### 15.1 Annexe A — Liste exhaustive des fichiers touchés

```
crates/efe-core/src/compute/backend.rs        [+50 LOC]
crates/efe-core/src/compute/cpu/mod.rs        [+30 LOC]
crates/efe-core/src/compute/cpu/linalg.rs     [+80 LOC] (l2_distance)
crates/efe-core/src/compute/cpu/consensus.rs  [+150 LOC] (cosine_*, blend)
crates/efe-core/src/compute/cpu/sampling.rs   [+60 LOC] (shannon_entropy)
crates/efe-core/src/compute/cpu/activ.rs      [+40 LOC] (sigmoid)
crates/efe-core/src/compute/cuda/mod.rs       [+30 LOC] (5 stubs)
crates/efe-core/src/compute/global.rs         NEW       [~30 LOC]
crates/efe-core/src/compute/utils.rs          NEW       [~50 LOC]
crates/efe-core/src/compute/mod.rs            [+5 LOC]

crates/efe-core/src/els/metrics.rs            [-150 LOC, +80 LOC]
crates/efe-core/src/els/projection.rs         [-100 LOC, +30 LOC]
crates/efe-core/src/genesis/meta_analyzer.rs  [-30 LOC, +10 LOC]
crates/efe-core/src/genesis/dag.rs            [-10 LOC, +5 LOC]
crates/efe-core/src/alignment/policy.rs       [-30 LOC, +15 LOC]
crates/efe-core/src/alignment/compose.rs      [-40 LOC, +15 LOC]
crates/efe-core/src/consensus/signals.rs      [-10 LOC, +5 LOC]
crates/efe-core/src/dissolution/alignment.rs  [-50 LOC, +20 LOC]
crates/efe-core/src/dissolution/decoders/nearest_anchor.rs [-20, +10]
crates/efe-core/src/dissolution/extractors/weighted_mean.rs [audit + ?]
crates/efe-core/src/fusion/kernels/softmax.rs    [-150, +30]
crates/efe-core/src/fusion/kernels/attention.rs  [-200, +50]
crates/efe-core/src/fusion/kernels/aggregate.rs  [-100, +30]
crates/efe-core/src/fusion/kernels/entropy.rs    [-150, +30]
crates/efe-core/src/fusion/kernels/project.rs    [-120, +30]

crates/efe-core/tests/compute_sprint5_units.rs       NEW [~400 LOC]
crates/efe-core/tests/compute_sprint5_cross_module.rs NEW [~600 LOC]
crates/efe-core/tests/compute_sprint5_golden.rs       NEW [~300 LOC]
crates/efe-core/tests/golden/l2_distance/basic.json   NEW
crates/efe-core/tests/golden/cosine_similarity/orthogonal.json NEW
crates/efe-core/tests/golden/cosine_anchor_blend/half_strength.json NEW
crates/efe-core/tests/golden/shannon_entropy/uniform.json NEW
crates/efe-core/tests/golden/sigmoid/reference_points.json NEW
crates/efe-core/tests/golden/cross_module/*.json (6 fichiers) NEW

docs/compute/tolerances.md                    [+ section 5]
docs/compute/sprint5_refactor_architecture.md NEW (ce document)
crates/efe-core/src/compute/ROADMAP.md        [Phase 1 cochée]
```

### 15.2 Annexe B — Schémas types

#### Schéma type d'une migration de façade

```rust
// AVANT (els/metrics.rs)
#[inline]
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "cosine_similarity length mismatch");
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    if denom < f32::EPSILON { 0.0 } else { dot / denom }
}

// APRÈS (els/metrics.rs)
#[inline]
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "cosine_similarity length mismatch");
    let backend = crate::compute::global::global_backend();
    let ta = Tensor::<f32>::from_slice(a, [a.len()].into())
        .expect("els::cosine_similarity: tensor construction never fails on valid slice");
    let tb = Tensor::<f32>::from_slice(b, [b.len()].into())
        .expect("els::cosine_similarity: tensor construction never fails on valid slice");
    backend.cosine_similarity_pair(&ta, &tb).unwrap_or(0.0)
}
```

#### Schéma type d'une nouvelle op `ComputeBackend`

```rust
// crates/efe-core/src/compute/cpu/consensus.rs

/// Cosine similarity between two rank-1 vectors.
///
/// # Returns
///
/// `dot(a, b) / (‖a‖₂ · ‖b‖₂)` capped at 1 in absolute value.
///
/// Returns `0.0` if either vector has near-zero L2 norm
/// (`‖a‖·‖b‖ < f32::EPSILON`), matching the geometric limit.
///
/// # Errors
///
/// * [`ComputeError::ShapeMismatch`] if either input is not rank 1
///   or the lengths differ.
/// * [`ComputeError::NumericalInstability`] if any input is non-finite.
pub fn cosine_similarity_pair(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
) -> Result<f32> {
    if a.shape().rank() != 1 || b.shape().rank() != 1 {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_similarity_pair",
            expected: "both rank 1".into(),
            actual: format!("a.rank={}, b.rank={}",
                a.shape().rank(), b.shape().rank()),
        });
    }
    if a.shape().dims() != b.shape().dims() {
        return Err(ComputeError::ShapeMismatch {
            op: "cosine_similarity_pair",
            expected: format!("matching length {}", a.shape().dims()[0]),
            actual: format!("{}", b.shape().dims()[0]),
        });
    }
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (&x, &y) in sa.iter().zip(sb.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(ComputeError::NumericalInstability {
                op: "cosine_similarity_pair",
                detail: "non-finite input element".into(),
            });
        }
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    Ok(if denom < f32::EPSILON { 0.0 } else { dot / denom })
}
```

### 15.3 Annexe C — Pseudo-code du test source-tree

```rust
// crates/efe-core/tests/s5_no_artisanal_numeric_ops.rs
//
// Source-tree regression latch : ensures no module outside compute/
// re-implements operations that are now provided by ComputeBackend.

use std::path::PathBuf;
use walkdir::WalkDir;
use regex::Regex;

const FORBIDDEN_PATTERNS: &[(&str, &str)] = &[
    ("softmax", r"fn\s+softmax\s*\("),
    ("attention", r"fn\s+(scaled_dot_product_)?attention\s*\("),
    ("dot_product",
        r"fn\s+dot(_product)?\s*\(\s*\w+\s*:\s*&\s*\[\s*f32\s*\]"),
    ("cosine_similarity",
        r"fn\s+cosine_similarity\s*\("),
    ("l2_norm",
        r"fn\s+l2_norm\s*\(\s*\w+\s*:\s*&\s*\[\s*f32\s*\]"),
    ("sigmoid", r"fn\s+sigmoid\s*\("),
    ("shannon_entropy",
        r"fn\s+(shannon_)?entropy\s*\("),
];

const ALLOWED_DIRS: &[&str] = &[
    "compute/",            // canonical home
    "compute/cuda/",
    "tests/",              // tests can name them freely
    "benches/",            // benches too
];

#[test]
fn no_artisanal_numeric_op_outside_compute() {
    let root = PathBuf::from("crates/efe-core/src");
    let mut violations: Vec<(String, String, usize)> = Vec::new();

    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "rs") { continue; }
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy();
        if ALLOWED_DIRS.iter().any(|d| rel.starts_with(d)) { continue; }

        let src = std::fs::read_to_string(path).unwrap();
        for (name, pattern) in FORBIDDEN_PATTERNS {
            let re = Regex::new(pattern).unwrap();
            for (line_idx, line) in src.lines().enumerate() {
                if re.is_match(line) && !line.trim_start().starts_with("//") {
                    violations.push(
                        (rel.to_string(), name.to_string(), line_idx + 1)
                    );
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "S5 violation: artisanal numeric ops detected outside compute/:\n{}",
        violations.iter()
            .map(|(p, n, l)| format!("  {p}:{l} → `{n}`"))
            .collect::<Vec<_>>().join("\n"),
    );
}
```

### 15.4 Annexe D — Décisions de design clés

| # | Décision | Alternative rejetée | Raison |
|---|----------|---------------------|--------|
| D1 | Backend singleton via `OnceLock` | Injection partout via paramètre | Préserve les signatures publiques `els::metrics` (~10 call sites externes) |
| D2 | Garder façade panic-on-mismatch | Renvoyer `Result` partout | Casse incompatibilité avec les call sites existants ; le panic est documenté contractuellement |
| D3 | bf16 hors scope S5 | Supporter bf16 dans les nouvelles ops | bf16 n'est pas dans `ComputeBackend` aujourd'hui ; ajouter S5 = +scope, +tests, +risque ; reporté en S7 |
| D4 | 5 nouvelles ops vs 4 (sans `cosine_anchor_blend`) | Émuler en composant `cosine_similarity_pair` + `weighted_mean` | 3 sites consommateurs identifiés (`els::projection`, `dissolution::alignment`, `fusion::project`) ⇒ rentable de l'élever |
| D5 | `pairwise_cosine` en helper interne, pas dans le trait | L'élever au trait | 2 sites consommateurs seulement (`alignment::policy I5`, `compose`) ; pas critique pour Phase 2 ; helper interne suffit |
| D6 | Garder l'API `Vec<f32>` des consommateurs | Migrer vers `Tensor<f32>` partout | Trop intrusif ; ferait exploser le scope. Conversion au point d'appel est acceptable |
| D7 | 1 PR par phase Ai | Une seule PR géante | Reviewabilité, granularité de rollback, possibilité de paralléliser A4-A6 |
| D8 | Test source-tree (regex) | Test runtime (introspection) | Plus simple, plus déterministe, attrape même les fonctions privées |
| D9 | Goldens cross-module en JSON | Goldens en code Rust | Cohérent avec l'infrastructure golden Sprint 3 ; éditables sans recompilation |
| D10 | Sprint 5 = 11 jours | Découper en S5 + S6 (2×6 jours) | Pas de logique fonctionnelle qui justifie la coupure ; tout est cohérent |

---

## Conclusion — pourquoi cette architecture

Cette architecture optimise simultanément **trois axes** :

1. **Conformité** au critère de sortie Phase 1 explicitement listé
   dans `compute/ROADMAP.md` ligne 645-647.
2. **Levier** maximal pour la Phase 2 CUDA : un seul effort de
   refactor, ×7 modules basculent automatiquement quand le backend
   passe à CUDA.
3. **Continuité** avec le pattern audit-friendly que les 5 audits
   Sprint 3 ont validé : chaque phase a son test gate, ses
   bisimulations, ses goldens vivants.

À l'issue du Sprint 5, **8 mois** d'efforts compute (S1-S5) sont
**capitalisés** dans une couche unique, testée à plus de 1 320
checks, prête à recevoir la Phase 2 CUDA d'un seul porting effort.

— *Fin du document Sprint 5.*

---

*Pour démarrer l'implémentation, voir le commit suivant qui ouvre
la Phase A1 avec les 5 nouvelles ops `ComputeBackend`.*
