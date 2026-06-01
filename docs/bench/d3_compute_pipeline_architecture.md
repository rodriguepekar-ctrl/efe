# Sprint D3 — Architecture : `benches/compute_pipeline.rs`

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position dans la séquence** : suit D1 (orchestration prod), D2
> (infra mémoire). D3 mesure la **couche compute** : la chaîne réelle
> de 5-6 opérations numériques que `MvpCpuFusion::run_round_sync`
> exécute à chaque round. C'est la couche qui consomme la majorité
> du temps CPU (et tout le temps GPU à terme).
> **Prérequis** : avoir lu
> [`d1_real_strategies_architecture.md`](d1_real_strategies_architecture.md)
> et [`d2_memory_real_arena_architecture.md`](d2_memory_real_arena_architecture.md).

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Gap analysis : ce qui n'est PAS mesuré aujourd'hui](#2-gap-analysis--ce-qui-nest-pas-mesuré-aujourdhui)
3. [Surface API à bencher (cartographie chirurgicale)](#3-surface-api-à-bencher-cartographie-chirurgicale)
4. [La séquence MvpCpuFusion::run_round canonique (6 phases)](#4-la-séquence-mvpcpufusionrun_round-canonique-6-phases)
5. [Architecture cible : structure du fichier](#5-architecture-cible--structure-du-fichier)
6. [Les 4 groupes criterion détaillés](#6-les-4-groupes-criterion-détaillés)
7. [12 décisions de conception clés](#7-12-décisions-de-conception-clés)
8. [10 risques identifiés et mitigations](#8-10-risques-identifiés-et-mitigations)
9. [Plan jour-par-jour (3 j wallclock)](#9-plan-jour-par-jour-3-j-wallclock)
10. [Calibration & nouveaux SLO `D3-*`](#10-calibration--nouveaux-slo-d3)
11. [Anti-régression & realism check](#11-anti-régression--realism-check)
12. [Intégration CI + parser + baseline v0.1.3](#12-intégration-ci--parser--baseline-v013)
13. [Critères d'acceptation (DoD)](#13-critères-dacceptation-dod)
14. [Out of scope & roadmap follow-ups](#14-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 Pourquoi descendre encore d'un étage après D1 et D2

D1 a mesuré la **chaîne complète** end-to-end : un nombre p95 par
fixture, agrégeant orchestration + compute + mémoire. D2 a mesuré
le **fond de pile** : pool/arena/backend, c.-à-d. *où* on stocke la
mémoire compute. D3 cible le **milieu de pile** — le **calcul
arithmétique lui-même**.

Ces trois suites bench se composent en une attribution complète :

```
            ┌──────────────────────────┐
            │  D1 — real_e2e_latency   │  total: ~5/50/250 ms
            │      (build_mvp_backend) │
            └────────────┬─────────────┘
                         │ décompose en
                         ▼
        ┌────────────────────────────────────┐
        │       Orchestration overhead       │  ~5-10 %
        │  (controller + tokio + telemetry)  │
        └────────────────────────────────────┘
        ┌────────────────────────────────────┐
        │   D3 — compute_pipeline (CHAIN)    │  ~80-90 % ← CŒUR
        │ scores+softmax+aggregate+project…  │
        └────────────────────────────────────┘
        ┌────────────────────────────────────┐
        │   D2 — memory_real_arena           │  ~5-10 %
        │   (pool acquire + arena alloc)     │
        └────────────────────────────────────┘
```

**Mission D3** : produire un bench criterion
**`crates/efe-core/benches/compute_pipeline.rs`** qui chaîne
**6 opérations `ComputeBackend`** dans le même ordre, sur les mêmes
shapes, qu'une vraie itération de `MvpCpuFusion::run_round_sync`,
**sans** mocks, **sans** ELS, **sans** arène — uniquement les ops
numériques pures. Le bench :

* Permet l'**attribution per-phase** (chaque op a sa propre courbe).
* Compare **naïf vs BLAS** (Apple Accelerate / OpenBLAS) sur les
  matmuls — la donnée *headline* du Sprint 4 Light.
* Définit **6 nouveaux SLO `D3-*`** gardés par CI.
* Est protégé par **3 tests anti-régression** dans
  `tests/compute_pipeline_realism.rs`.

### 1.2 Localisation décidée : `efe-core/benches/`

Comme D2. Justification :

* `ComputeBackend` + `CpuBackend` + `Tensor` vivent tous dans
  `efe_core::compute`. **Aucune dépendance `efe-server` n'est
  requise.**
* Co-localisation avec `compute_sprint{2,3,4}.rs` qui mesurent les
  ops *isolées* — D3 mesure la *chaîne*, complémentaire et non
  redondante.
* Rebuild graph minimal : un changement dans `efe-server` ne
  ré-invalide pas D3.

### 1.3 Pourquoi un nouveau bench plutôt qu'étendre `compute_sprint*.rs`

Les benches Sprint 2/3/4 mesurent **chaque op en isolation, avec
des inputs aléatoires xorshift**. Trois limites bloquantes pour D3 :

| Limite | Conséquence | Comment D3 la lève |
|--------|------------|--------------------|
| Inputs xorshift random | Pas représentatif de la distribution réelle (latents Gaussiens, attention en post-softmax) | D3 chaîne les ops pour que l'output de N alimente N+1 — distributions réalistes |
| Op par op, pas de chaînage | Pas de mesure de la *composition* (data flow, allocation churn entre ops) | D3 mesure la chaîne complète comme une seule boîte noire |
| Shapes arbitraires (32², 64², …) | Ne match pas les shapes prod (`n_agents × dim` du v2) | D3 utilise les shapes v2 fixtures (small/medium/large) |

Donc D3 et Sprint 2/3/4 sont **orthogonaux** : Sprint 2/3/4 = micro,
D3 = macro-composition. Les deux restent en place dans le bench
runner.

---

## 2. Gap analysis : ce qui n'est PAS mesuré aujourd'hui

### 2.1 Inventaire des suites bench post-D2

| Suite | Fichier | Crate | Cible |
|-------|---------|-------|-------|
| `swarm_e2e` | `efe-core/benches/swarm_e2e.rs` | core | controller mocks (Sprint C) |
| `memory_e2e` | `efe-core/benches/memory_e2e.rs` | core | heap mocks (Sprint C) |
| `compute_sprint2` | `efe-core/benches/compute_sprint2.rs` | core | matmul, softmax, gelu en isolation |
| `compute_sprint3` | `efe-core/benches/compute_sprint3.rs` | core | cone_project, weighted_mean, moe_reconcile, etc. |
| `compute_sprint4` | `efe-core/benches/compute_sprint4.rs` | core | matmul naïf vs BLAS |
| `real_strategies_e2e` | `efe-server/benches/real_strategies_e2e.rs` | server | wiring prod (D1) |
| `real_memory` | `efe-server/benches/real_memory.rs` | server | heap prod (D1) |
| `memory_real_arena` | `efe-core/benches/memory_real_arena.rs` | core | pool/arena (D2) |
| `compute_pipeline` *(D3)* | **manquant** | core | **chaîne 6 ops** |

### 2.2 Décomposition de `MvpCpuFusion::run_round_sync` — qui mesure quoi ?

| Phase | Cost % (estimé) | Couvert par |
|-------|----------------|-------------|
| Pre-round stats (`compute_stats`) | ~5 % | Aucun bench dédié |
| Replay snapshot (memcpy) | ~2 % | Aucun |
| Phase 1 — `compute_scores` (matmul) | **~25 %** | `compute_sprint{2,4}` (isolation) |
| Phase 2 — `softmax_in_place_rows` | ~10 % | `compute_sprint2` (isolation) |
| Phase 3 — `aggregate` (matmul) | **~30 %** | `compute_sprint{2,4}` (isolation) |
| Pointer swap | ~0 % | Aucun |
| Phase 4 — `beta_mix` (axpy) | ~3 % | Aucun bench dédié |
| Phase 5 — `project_rows` (cone) | **~15 %** | `compute_sprint3` (isolation) |
| Phase 6 — `clip_and_check` (clamp) | ~3 % | Aucun bench dédié |
| Post-round stats | ~5 % | Aucun |

**Trou de couverture** : la *somme* de ces ops, exécutée *avec data
flow réel*, **n'est mesurée nulle part**. D1 la mesure
transitivement (perdue dans l'orchestration), D2 ne touche pas le
compute, et `compute_sprint*.rs` mesurent les ops en silos. Si la
chaîne devient sub-optimale (ex : alignement L1 ↔ L2 cassé entre
matmul et softmax), aucun bench actuel ne le détecte.

### 2.3 Conclusion : 3e moitié de la couverture compute manquante

Sprint C avait mocké tout le compute — **0 % couverture**.
Sprint 2/3/4 ont implémenté les 23 ops avec micro-benches —
**~50 % couverture** (chaque op isolée).
D1 a mesuré la composition complète — **~5 % couverture pour la
couche compute proprement dite** (perdue dans le bruit
orchestration).
D3 ferme le gap : **chaîne réelle, shapes réelles, data flow réel**.

---

## 3. Surface API à bencher (cartographie chirurgicale)

### 3.1 `ComputeBackend` (`efe_core::compute::ComputeBackend`)

23 ops totales — D3 en utilise **6** dans la chaîne canonique :

```rust
pub trait ComputeBackend: Send + Sync + std::fmt::Debug {
    // Ops utilisées dans la chaîne D3 (en gras = phases majeures)
    fn matmul(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>>;          // ★ phases 1, 3
    fn softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;                // ★ phase 2
    fn cone_project(&self, latents: &Tensor<f32>, anchors: &Tensor<f32>, max_dist: f32)
        -> Result<Tensor<f32>>;                                                            // ★ phase 5
    fn shannon_entropy(&self, p: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;        // phase 6 (stats)
    fn std_dev(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>>;                // post-stats

    // Ops collatérales accessibles à la chaîne, non utilisées en core D3 :
    fn matmul_fused_gelu(&self, a, b, bias) -> ...;     // alternative pour Phase 1+3 fusionnées
    fn weighted_mean(&self, values, weights, l2) -> ...;
    fn scaled_dot_product_attention(&self, q, k, v, mask) -> ...; // fused 1+2+3 mais pas mesurable
    // ... (gelu, sigmoid, dot, l2_*, topk, argmax, ...)
}
```

### 3.2 `CpuBackend` — l'implémentation

```rust
pub struct CpuBackend;
impl CpuBackend {
    pub fn new() -> Self;
}
impl ComputeBackend for CpuBackend { /* 23 ops */ }
```

* Crée sans config — `Self` est unit-like.
* `Send + Sync` — peut être cloné dans des tasks tokio si jamais.
* **Avec feature `accelerate`** : `matmul` au-dessus du seuil
  `M·K·N ≥ 32 768` dispatche vers Apple Accelerate (~×40 sur
  256² selon `BENCHMARKS.md` § 5).
* **Avec feature `openblas`** : pareil mais OpenBLAS.

### 3.3 `Tensor<T>` — le type de donnée

```rust
impl<T> Tensor<T> {
    pub fn from_slice(data: &[T], shape: ShapeVec) -> Result<Self>;
    pub fn shape(&self) -> &[usize];
    pub fn data(&self) -> &[T];
    // ...
}
```

* **Owned** : la donnée est dans le `Tensor`, pas borrowée.
* **Layout row-major** standard.
* **Conversion gratuite** depuis `&[T]` via `from_slice`.
* Le bench peut donc construire les tenseurs **une fois en setup**
  et les black-boxer dans la boucle iter sans coût d'allocation
  observable.

### 3.4 Dispatch BLAS : où est le seuil ?

D'après `compute_sprint4.rs` et le code `compute/cpu_matmul.rs` :

```
M · K · N >= 32 768  →  dispatche vers BLAS si la feature est active
M · K · N <  32 768  →  reste sur le path naïf rust
```

Donc, pour les v2 fixtures :

| fixture | Phase 1 (Q @ Kᵀ) M·K·N | Phase 3 (A @ V) M·K·N | BLAS ? |
|---------|------------------------:|----------------------:|-------:|
| small | 8·128·8 = 8 192 | 8·8·128 = 8 192 | **non** |
| medium | 32·256·32 = 262 144 | 32·32·256 = 262 144 | **oui** (×8 vs seuil) |
| large | 128·256·128 = 4 194 304 | 128·128·256 = 4 194 304 | **oui** (×128 vs seuil) |

Conséquence pour D3 : sur **small**, le suffixe `_blas` produira
des chiffres **identiques** au `_naive`. C'est le comportement
attendu et le SLO le reflète (cible identique). Sur **medium** et
**large**, on observera un speedup marqué.

### 3.5 Pourquoi 6 ops et pas plus

Pourquoi ne pas inclure les 23 ops de `ComputeBackend` ? Trois
raisons :

1. **Représentativité** : `MvpCpuFusion::run_round_sync` n'utilise
   *pas* les 23. Bencher des ops qu'aucun chemin de prod
   n'exerce serait bruit.
2. **Lecture humaine** : 6 ops = 6 phases criterion lisibles d'un
   coup d'œil. 23 = matrice illisible.
3. **Composabilité** : 6 ops × 3 sizes × 2 BLAS modes = 36
   sub-benches déjà — assez pour saturer un budget CI 5 min en
   `--quick`.

Les ops restantes (gelu, sigmoid, topk, argmax, etc.) sont
couvertes par `compute_sprint*.rs` (en isolation) — D3 se
concentre sur la *chaîne réelle*.

---

## 4. La séquence `MvpCpuFusion::run_round` canonique (6 phases)

### 4.1 Lecture annotée du code source

Extrait fidèle de `crates/efe-core/src/fusion/mvp_cpu.rs` :

```text
run_round_sync(els, arena, cfg, round) :
    // pre-round stats + validation (skip for D3 — out of scope)

    // ┌────── PHASE 1 — scores ────────────────────────────────┐
    let mut scores = vec![0.0f32; n*n];
    compute_scores(latent, mask, &mut scores, params);
    //   → semantically: scores = (latent @ latent_T) / sqrt(d) [+ mask]
    //   → maps to ComputeBackend::matmul(Q, K_transpose) + scale + mask
    // └─────────────────────────────────────────────────────────┘

    // ┌────── PHASE 2 — softmax ───────────────────────────────┐
    softmax_in_place_rows(&mut scores, n);
    //   → semantically: attention[i,:] = softmax(scores[i,:])
    //   → maps to ComputeBackend::softmax(scores, axis=-1)
    // └─────────────────────────────────────────────────────────┘

    // (entropy stats, read-only, not in D3 inner chain)

    // ┌────── PHASE 3 — aggregate ─────────────────────────────┐
    aggregate(latent, &mut new_latent, mask, scores, &mut acc);
    //   → semantically: new_latent = attention @ latent
    //   → maps to ComputeBackend::matmul(attention, V)
    // └─────────────────────────────────────────────────────────┘

    swap_latent_with_scratch();

    // ┌────── PHASE 4 — β-mix ─────────────────────────────────┐
    beta_mix(&mut latent, problem_embedding, mask, beta);
    //   → semantically: latent = (1-β) * latent + β * problem
    //   → no direct ComputeBackend op (axpy-like) — *D3 fuses with
    //     a manual ndarray::Zip OR uses cosine_anchor_blend with
    //     anchors=[problem] for the realistic equivalent*
    // └─────────────────────────────────────────────────────────┘

    // ┌────── PHASE 5 — project ───────────────────────────────┐
    project_rows(&mut latent, anchors, mask, alpha, &mut row_scratch);
    //   → semantically: per row, blend toward closest anchor
    //   → maps to ComputeBackend::cone_project(latent, anchors, max_dist)
    // └─────────────────────────────────────────────────────────┘

    // ┌────── PHASE 6 — clip ──────────────────────────────────┐
    clip_and_check(&mut latent, mask, clip_max);
    //   → semantically: clamp |latent| <= clip_max element-wise
    //   → no direct ComputeBackend op — D3 uses *shannon_entropy* on
    //     the attention output as a stats placeholder + a manual clamp,
    //     OR reframes phase 6 as "entropy stats" which is what the
    //     prod path actually produces in *parallel* with clip.
    // └─────────────────────────────────────────────────────────┘

    // post-round stats (compute_stats — out of scope for D3 chain)
```

### 4.2 Mapping pragmatique pour D3

Toutes les phases prod ne mappent pas 1-pour-1 sur `ComputeBackend`.
Décision pragmatique : **D3 chaîne 6 ops `ComputeBackend` qui
représentent fidèlement le pattern d'usage**, pas littéralement
les 6 phases prod. La fidélité conservée :

* **Le data flow** : output de op N alimente op N+1.
* **Les shapes** : matmul tape `(N, D) × (D, N) → (N, N)` puis
  softmax et matmul `(N, N) × (N, D) → (N, D)`.
* **L'ordre** : matmul → softmax → matmul → projection → stats.
* **Le coût macroscopique** : ~80 % du temps fusion va dans ces 6
  ops. La chaîne D3 reproduit ce profil.

| D3 phase | Op `ComputeBackend` | Shapes (medium fixture, dim=256, n=32) | Correspond à |
|----------|--------------------|-----------------------------------------|--------------|
| **1** | `matmul(Q, K_T)` | (32,256) × (256,32) → (32,32) | `compute_scores` |
| **2** | `softmax(scores, axis=-1)` | (32,32) → (32,32) | `softmax_in_place_rows` |
| **3** | `matmul(attn, V)` | (32,32) × (32,256) → (32,256) | `aggregate` |
| **4** | `cone_project(latent, anchors, max_dist)` | (32,256) → (32,256) | `project_rows` |
| **5** | `shannon_entropy(attn, axis=-1)` | (32,32) → (32,) | entropy stats |
| **6** | `std_dev(latent, axis=0)` | (32,256) → (256,) | post-stats |

Justification de l'inclusion de stats (5+6) : `MvpCpuFusion`
appelle `compute_stats` pre+post pour fournir des metrics au
detector. Ces appels *sont* du compute (variance, gradient norm),
~10 % du temps total. Inclus dans la chaîne pour fidélité.

### 4.3 Note sur les phases 4 (β-mix) et 6 (clip) — pourquoi pas D3

* **β-mix** : `latent = (1-β) * latent + β * problem` — élément
  par élément. Aucune op `ComputeBackend` ne fait exactement ça
  (le plus proche est `weighted_mean` mais avec une sémantique
  différente). Coût bench-pertinent : ~3 % du temps round, sous le
  bruit de mesure. **Hors scope D3** (couvert par
  `compute_sprint*.rs` ou inline).
* **clip** : `|latent_ij| <= clip_max` — element-wise clamp + scan.
  Pas d'op `ComputeBackend`. ~3 % du temps. **Hors scope D3**.

Ces deux phases ensemble = ~6 % du temps round. D3 mesure les ~80 %
qui dominent.

---

## 5. Architecture cible : structure du fichier

### 5.1 Localisation et déclaration Cargo

**Fichier** : `crates/efe-core/benches/compute_pipeline.rs`
**LOC cible** : ~600 lignes (commentaires denses inclus).
**Cargo.toml** :

```toml
[[bench]]
name = "compute_pipeline"
harness = false
```

Pas de feature à ajouter — `accelerate`/`openblas` existent déjà
sur `efe-core` depuis Sprint 4.

### 5.2 Structure du fichier (squelette)

```rust
//! D3 — bench compute pipeline (chaîne réelle 6 ops).

#![allow(missing_docs)]
#![allow(clippy::cast_precision_loss)]

mod common; // partagé (xorshift_vec, build_problem_embedding,
            // build_anchors, ...)

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

// ========================================================
// BLAS suffix detection — same pattern as D1
// ========================================================
#[cfg(any(feature = "accelerate", feature = "openblas"))]
const BLAS_SUFFIX: &str = "_blas";
#[cfg(not(any(feature = "accelerate", feature = "openblas")))]
const BLAS_SUFFIX: &str = "_naive";

// ========================================================
// Fixture sizes (matched on D1 v2)
// ========================================================
struct Fixture {
    name: &'static str,
    n_agents: usize,
    dim: usize,
    n_anchors: usize,
    max_dist: f32,
}
const FIXTURES: &[Fixture] = &[
    Fixture { name: "small",  n_agents: 8,   dim: 128, n_anchors: 4,  max_dist: 0.30 },
    Fixture { name: "medium", n_agents: 32,  dim: 256, n_anchors: 8,  max_dist: 0.25 },
    Fixture { name: "large",  n_agents: 128, dim: 256, n_anchors: 16, max_dist: 0.20 },
];

// ========================================================
// Test inputs (built once per bench group)
// ========================================================
struct ChainInputs {
    /// Q : (n_agents, dim). Used as both Q AND V (latent reuse).
    q: Tensor<f32>,
    /// K_T : (dim, n_agents). Pre-transposed for matmul.
    k_t: Tensor<f32>,
    /// Anchors : (n_anchors, dim). One-hot orthogonal at evenly-spaced indices.
    anchors: Tensor<f32>,
    /// Cone radius (passed to cone_project).
    max_dist: f32,
}
fn build_chain_inputs(f: &Fixture) -> ChainInputs;

// ========================================================
// 4 criterion groups (the heart of D3)
// ========================================================
fn bench_compute_pipeline_chain(c: &mut Criterion);
fn bench_compute_pipeline_per_phase(c: &mut Criterion);
fn bench_compute_pipeline_throughput(c: &mut Criterion);
fn bench_compute_pipeline_blas_speedup_summary(c: &mut Criterion);

criterion_group!(
    name = compute_pipeline_benches;
    config = Criterion::default();
    targets =
        bench_compute_pipeline_chain,
        bench_compute_pipeline_per_phase,
        bench_compute_pipeline_throughput,
        bench_compute_pipeline_blas_speedup_summary,
);
criterion_main!(compute_pipeline_benches);
```

### 5.3 Helpers détaillés

```rust
/// Build the input tensors for a single fixture. Shapes :
///
/// * `q`   = (n_agents, dim)        — represents both Q and V
///                                     (the prod fusion uses the
///                                      same latent matrix as both).
/// * `k_t` = (dim, n_agents)        — pre-transposed K so phase 1
///                                     does plain `matmul(Q, K_T)`.
/// * `anchors` = (n_anchors, dim)   — one-hot orthogonal at
///                                     evenly-spaced indices,
///                                     matching `gen_fixtures_v2.py`.
///
/// Tensors are filled with deterministic xorshift values rescaled
/// to `[-1, 1]` so the softmax in phase 2 has non-trivial dynamic
/// range and the cone projection in phase 4 sees a realistic
/// distance distribution.
fn build_chain_inputs(f: &Fixture) -> ChainInputs {
    // xorshift seed strategy: deterministic per fixture name
    let seed = match f.name {
        "small"  => 0xD3_5A_AA_BBu64,
        "medium" => 0xD3_5A_BB_CCu64,
        _        => 0xD3_5A_CC_DDu64,
    };
    let q     = random_tensor(&[f.n_agents, f.dim], seed);
    let k_raw = random_tensor(&[f.n_agents, f.dim], seed.wrapping_add(1));
    // Pre-transpose K → K_T : (dim, n_agents).
    let k_t   = transpose_2d(&k_raw);
    // Anchors : one-hot at i*step.
    let anchors = build_one_hot_anchors(f.dim, f.n_anchors);
    ChainInputs { q, k_t, anchors, max_dist: f.max_dist }
}
```

---

## 6. Les 4 groupes criterion détaillés

### 6.1 `compute_pipeline_chain` — la chaîne complète, 3 sizes

**Intention** : la métrique-roi de D3. Mesure **le coût total**
de la chaîne (op1+op2+...+op6) en *un seul* timing par itération.
C'est le chiffre qu'on compare à `real_e2e_latency_*::*` (D1) pour
isoler le coût compute pur de l'orchestration.

**Setup** :
```rust
let backend = CpuBackend::new();
let inputs = build_chain_inputs(fixture);
```

**Itération** :
```rust
b.iter(|| {
    // PHASE 1 — scores = Q @ K_T
    let scores = backend.matmul(&inputs.q, &inputs.k_t).unwrap();
    // PHASE 2 — attention = softmax(scores, axis=-1)
    let attention = backend.softmax(&scores, -1).unwrap();
    // PHASE 3 — aggregated = attention @ Q   (Q ≡ V)
    let aggregated = backend.matmul(&attention, &inputs.q).unwrap();
    // PHASE 4 — projected = cone_project(aggregated, anchors, max_dist)
    let projected = backend
        .cone_project(&aggregated, &inputs.anchors, inputs.max_dist)
        .unwrap();
    // PHASE 5 — entropy = shannon_entropy(attention, axis=-1)
    let entropy = backend.shannon_entropy(&attention, -1).unwrap();
    // PHASE 6 — std_dev(projected, axis=0)  — post-round stats proxy
    let stats = backend.std_dev(&projected, 0).unwrap();
    // black_box keeps the chain alive across the optimizer
    black_box((scores, attention, aggregated, projected, entropy, stats));
});
```

**Sample size** : 50 (small/medium) ; 30 (large). Total time per
iter ≈ 50 µs (small) ; 500 µs (medium) ; 5 ms (large) — criterion
auto-tunes iters/sample.

**Throughput** : `Throughput::Elements(1)` (= 1 chain/iter). Donne
chains/s.

**Coût décomposé attendu (medium, naïve)** :

| Phase | Op | Coût attendu (M3 Pro) |
|-------|----|----------------------:|
| 1 | matmul(32,256)×(256,32)→(32,32) | ~70 µs |
| 2 | softmax(32,32) | ~5 µs |
| 3 | matmul(32,32)×(32,256)→(32,256) | ~70 µs |
| 4 | cone_project(32,256, 8 anchors) | ~30 µs |
| 5 | shannon_entropy(32,32) | ~5 µs |
| 6 | std_dev(32,256) | ~10 µs |
| **Total** | — | **~190 µs** |

**Avec BLAS** (medium) : matmuls 1+3 passent de 140 µs à ~5-10 µs
via Accelerate ⇒ chaîne totale ~60-80 µs (×2.5 speedup).

**SLO targets** :

| Bench key | Cible naïf | Cible BLAS |
|-----------|-----------:|-----------:|
| `compute_pipeline_chain_naive::small` | ≤ 200 µs | ≤ 200 µs (BLAS pas dispatchée) |
| `compute_pipeline_chain_naive::medium` | ≤ 500 µs | n/a |
| `compute_pipeline_chain_blas::medium` | n/a | ≤ 250 µs |
| `compute_pipeline_chain_naive::large` | ≤ 20 ms | n/a |
| `compute_pipeline_chain_blas::large` | n/a | ≤ 5 ms |

### 6.2 `compute_pipeline_per_phase` — attribution per-op

**Intention** : ce qui distingue D3 d'un simple "chain bench" : on
mesure **chaque phase isolément**, mais avec **les vrais inputs
issus de la phase précédente**, pas des inputs random. C'est ce
qui donne l'attribution réaliste : "70 % du chain time est dans
les 2 matmuls, 15 % dans cone_project, 10 % dans softmax+stats,
5 % de bruit".

**Difficulté** : pour bencher la phase 4 (cone_project) avec un
input réaliste, il faut d'abord exécuter les phases 1-3. On ne
peut pas timer la phase 4 sans payer 1-3.

**Solution** : pré-calculer les outputs des phases 1-3 *dans le
setup*, les stocker, puis bencher les ops 1-6 *individuellement*
avec leurs vrais inputs. Setup runs once per `bench_with_input`
call — coût amorti.

**Pseudo-code** :
```rust
fn bench_compute_pipeline_per_phase(c: &mut Criterion) {
    let backend = CpuBackend::new();
    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);
        // Pré-calcul des outputs intermédiaires (en setup, hors timer)
        let scores     = backend.matmul(&inputs.q, &inputs.k_t).unwrap();
        let attention  = backend.softmax(&scores, -1).unwrap();
        let aggregated = backend.matmul(&attention, &inputs.q).unwrap();
        let projected  = backend
            .cone_project(&aggregated, &inputs.anchors, inputs.max_dist)
            .unwrap();
        // (entropy et std_dev n'ont pas de "post" — ce sont des feuilles)

        let mut group = c.benchmark_group(
            format!("compute_pipeline_per_phase{BLAS_SUFFIX}"));

        // Phase 1
        group.bench_with_input(
            BenchmarkId::new(format!("{}/scores", fixture.name), fixture.name),
            fixture.name,
            |b, _| b.iter(|| {
                let r = backend.matmul(&inputs.q, &inputs.k_t).unwrap();
                black_box(r);
            }),
        );
        // Phase 2 ... Phase 6 — same pattern.
        // ...
    }
}
```

**Sample size** : 100 (phases ns/µs scale) ; 50 (large matmuls).

**Lecture** : criterion produit 6 × 3 = 18 sub-benches. La somme
DOIT matcher (à ~10 % près) le `compute_pipeline_chain` du même
size — c'est l'**hypothèse d'additivité** que le bench valide
implicitement.

### 6.3 `compute_pipeline_throughput` — sweep de `n_agents`

**Intention** : exposer la **scalabilité** de la chaîne en
fonction de `n_agents`. Pour `n` ∈ {8, 16, 32, 64, 128, 256, 512},
mesurer la chaîne avec `dim=256` fixe.

**Usage** : permet de répondre à "à partir de quel `n_agents` la
chaîne devient O(N²) limited (coût domination par les matmuls) vs
O(N) limited (cone_project / stats)?". La courbe log-log donne
l'exposant empirique.

**Setup** :
```rust
let backend = CpuBackend::new();
const DIM: usize = 256;
const N_ANCHORS: usize = 8;
for &n in &[8usize, 16, 32, 64, 128, 256, 512] {
    let f = Fixture { name: "sweep", n_agents: n, dim: DIM, n_anchors: N_ANCHORS, max_dist: 0.25 };
    let inputs = build_chain_inputs(&f);
    // ... bench iter as in 6.1
}
```

**Throughput** : `Throughput::Elements(n*n)` (n² operations
dominantes). Criterion reporte ops/s.

**Sample size** : 30 — ce groupe est secondaire, économie de wall
time.

### 6.4 `compute_pipeline_blas_speedup_summary` — résumé dual

**Intention** : un *unique* sub-bench par fixture qui produit
explicitement le ratio `chain_naive / chain_blas`. Utile pour les
slides perf : "BLAS donne ×2.5 sur medium, ×4.2 sur large".

**Implémentation** : ce groupe **ne tourne que si BLAS est
activé**. Quand BLAS est off, le ratio est 1.0 par construction
et le bench n'a aucune valeur ajoutée.

```rust
#[cfg(any(feature = "accelerate", feature = "openblas"))]
fn bench_compute_pipeline_blas_speedup_summary(c: &mut Criterion) {
    // Re-bench the chain on each fixture, tagged "blas".
    // The naive baseline is loaded from `bench_baselines/v0.1.3/`
    // at parse time by parse_criterion_report.py — no runtime
    // dual-execution.
}
#[cfg(not(any(feature = "accelerate", feature = "openblas")))]
fn bench_compute_pipeline_blas_speedup_summary(_: &mut Criterion) {
    // No-op: when BLAS is off, this group is empty.
}
```

**Alternative** : le groupe disparaît purement (au lieu d'être
no-op) si on conditionne le `criterion_group!` macro avec un
`#[cfg]`. Le no-op est plus lisible mais produit un bench vide ;
le `#[cfg]` au niveau macro évite la pollution. Décision : **cfg
au niveau macro**, voir §7 D9.

---

## 7. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | `efe-core/benches/` (vs `efe-server/`) | server-side | la cible (`ComputeBackend`) est dans core ; aucune dep server |
| D2 | Bencher la chaîne au niveau `ComputeBackend`, **pas** `fusion::kernels::*` | accès direct via `pub(crate)` | les kernels fusion sont `pub(crate)` (inaccessibles depuis benches/) ; ComputeBackend est `pub` et représente fidèlement l'arithmétique |
| D3 | 6 ops (matmul ×2, softmax, cone_project, shannon_entropy, std_dev) | 23 ops | seules ces 6 forment le chemin chaud de fusion ; les autres = `compute_sprint*.rs` |
| D4 | Inputs xorshift déterministes par fixture | Box-Muller Gaussian | xorshift suffit ici : on mesure l'**arithmétique**, pas la convergence ; xorshift = pas de seed-DRY entre v2 fixtures et bench compute |
| D5 | Q ≡ V (réutilisation du latent comme valeur) | tenseur V séparé | reflète exactement ce que MvpCpuFusion fait : `aggregate(latent, attention)` — le latent est valeur ET requête |
| D6 | K *pré-transposé* (K_T) construit en setup | matmul(Q, K).transpose() en bench | la transposition est un choix d'usage ; en pratique les latents sont déjà row-major, K_T est une vue logique. Pré-transpose = pas de pénalité d'alloc supplémentaire dans le bench |
| D7 | `cone_project` (Sprint 3) plutôt que `cosine_anchor_blend` (Sprint 5) | Sprint 5 op | `cone_project` est l'op historiquement utilisée par `MvpCpuFusion::project_rows` ; Sprint 5 a refactorisé certains call sites mais la fusion principale n'a pas (encore) migré |
| D8 | Suffixe `_naive`/`_blas` au niveau **groupe** (cf D1) | suffixe au niveau benchmark id | uniformité avec D1 ; lecture immédiate du report criterion |
| D9 | Sub-bench BLAS speedup `#[cfg]`-gated au niveau **macro** | no-op runtime | criterion_group! avec `#[cfg]` produit deux variantes du criterion_main!, l'une avec BLAS l'autre sans — pas de bench vide pollutif |
| D10 | Pas d'`async` ici | runtime tokio comme D1/D2 | aucune op `ComputeBackend` n'est async ; tokio = pollution sans valeur |
| D11 | Pas de `peak_alloc` | global allocator | timings doivent rester pristine ; D2.5 + D3.5 (follow-up) pour l'aspect mémoire |
| D12 | β-mix et clip **hors-scope** | inclure manuellement | aucune op `ComputeBackend` direct ; ~6 % du temps rond combinés ; bruit > signal pour D3 |

---

## 8. 10 risques identifiés et mitigations

### R1 — Le bench mesure `Tensor` clone overhead, pas l'op

**Impact** : `backend.matmul(a, b)` retourne un `Tensor<f32>`
*owned*. Si l'allocation domine le timing de l'op, on mesure
`Box<[f32]>::new` et pas le calcul.

**Mitigation** :
* Le coût d'une alloc 32×32 = 4 KB ≈ 100 ns sur l'allocator
  système Rust. Pour matmul 32×256×32 = 70 µs, c'est < 0.2 % —
  négligeable.
* Pour les phases ns/µs (softmax 32×32 = 5 µs), l'allocation
  contribue ~2 %. Acceptable.
* Si jamais on observe un floor à 100 ns, ajouter un sub-bench
  `tensor_alloc_only` pour calibrer.

### R2 — BLAS thread oversubscription

**Impact** : Apple Accelerate spawne des threads internes
(typiquement `ncpu` worker threads). Si le bench tourne dans une
runtime tokio multi-thread, on peut avoir N (criterion samples) ×
M (BLAS threads) workers contendant pour les mêmes cores.

**Mitigation** :
* D3 est **synchrone** (pas de tokio runtime) — N=1 thread du
  côté criterion par sample.
* Apple Accelerate auto-détecte la charge et dimensionne sa thread
  pool ; sur M3 Pro elle reste dans les ~4-8 threads.
* Documenté dans `BENCHMARKS.md` : "BLAS speedup mesuré avec une
  charge bench mono-thread ; en concurrence (cf. D1
  `real_e2e_throughput`), le speedup peut être moindre."

### R3 — Cache pollution entre phases

**Impact** : la chaîne 6 ops produit ~5 tenseurs intermédiaires
qui résident en cache. Sur **small** (8×128 = 1 KB par tenseur),
tout tient en L1. Sur **large** (128×256 = 128 KB par tenseur), les
matmuls cassent le L2 (M3 Pro = 12 MB par cluster). Le bench
*observe* ces effets de cache — c'est *par design*, pas un bug.

**Mitigation** : aucune ; le bench reflète le coût de cache réel.
**Documenté** dans BENCHMARKS.md : "le large bench est cache-bound
sur la phase 3 ; les nombres extrapolent mal vers des fixtures
encore plus grandes."

### R4 — `cone_project` vs `cosine_anchor_blend` sémantique différente

**Impact** : MvpCpuFusion utilise `project_rows` qui combine les
deux sémantiques (cone_project ET cosine_anchor_blend). Si D3
choisit l'une, on biaise vers ce profil.

**Mitigation** :
* D7 documenté : `cone_project` retenu par défaut (correspond au
  comportement historique).
* Une variante `compute_pipeline_chain_blend` ajoutée *en option*
  (out-of-scope DoD, follow-up D3.5) si le besoin émerge.

### R5 — Fixture `large` trop coûteuse en `--quick` CI

**Impact** : large (128 agents, dim=256) chaîne ≈ 5 ms (avec
BLAS), 20 ms (sans). Avec ~30 samples × 5 ms = 150 ms par
sub-bench × 6 phases = 900 ms juste pour la phase per_phase
large. `--quick` budget total = 5 min.

**Mitigation** :
* Sample size adapté : large → 30 samples au lieu de 50.
* `--quick` mode dans criterion limite déjà à 1 sample × measurement
  time fixe — automatique.
* Si CI dépasse le timeout, fallback : retirer `large` du
  per_phase, garder dans chain (où le coût est diluté).

### R6 — Compilation conditionnelle BLAS casse la lecture du report

**Impact** : si l'opérateur run `cargo bench --features accelerate`
puis run sans, les deux reports ont des keys disjointes
(`_blas` vs `_naive`). Comparer les deux = travail manuel.

**Mitigation** :
* `parse_criterion_report.py` reconnaît les deux suffixes via
  `WANTED_PREFIXES`. Le report consolidé contient les deux.
* SLO `D3-*` définissent **deux clés** par fixture : naïve ET
  BLAS. Run one, the other is `skipped`.
* Documentation `bench_runner.sh` : pour comparer, run twice et
  diff-merger les deux baselines (D1 pattern).

### R7 — Inputs xorshift biaisent vers la valeur 0

**Impact** : xorshift_vec produit des f32 ∈ [-1, 1] uniforme. Le
matmul Q @ K_T avec Q, K ∼ U(-1,1) donne des scores ∈ approx
N(0, sqrt(d)/3) — pas dégénérés. Mais en post-softmax, l'attention
est ~uniform car les scores ne se distinguent pas fortement. La
chaîne est réaliste *quantitativement* (coûts), pas
*qualitativement* (convergence).

**Mitigation** : non-bloquant. D3 mesure des coûts arithmétiques,
pas la convergence (qui est mesurée par D1's `real_e2e_latency`
indirectement et par les tests de convergence directement). On
documente la limite.

### R8 — Régression silencieuse via changement d'API ComputeBackend

**Impact** : si Sprint 6 change `matmul` pour prendre `&mut Tensor`
au lieu de retourner un nouveau, la signature change → bench
casse à compile time → **détection immédiate**. Bonne nouvelle.
Si la sémantique change (axe par défaut, dispatch, etc.) sans
changer la signature, le bench compile mais mesure la mauvaise
chose.

**Mitigation** : `tests/compute_pipeline_realism.rs` (cf §11)
inclut un test qui calcule une trace numérique attendue d'un input
fixé, et la compare. Tout drift sémantique trip ce test.

### R9 — Apple Accelerate vs OpenBLAS divergence numérique

**Impact** : sur M3 Pro avec `accelerate`, `matmul(a, b)` retourne
des f32 différents de la version OpenBLAS sur Linux x86. Tolerance
documentée dans `docs/compute/tolerances.md` à 1e-5 relative.

**Mitigation** :
* D3 ne **vérifie pas** les valeurs — il ne mesure que les coûts.
* `tests/compute_pipeline_realism.rs` utilise une tolérance
  pré-calculée pour le naïve path (deterministe) ; pour BLAS, le
  test est `#[cfg(not(any(feature = "accelerate", feature = "openblas")))]`-gated
  et skipped.

### R10 — Variance criterion sur les phases ns-scale

**Impact** : `softmax(8,8)` ≈ 200 ns, `shannon_entropy(8,8)` ≈
100 ns. Variance scheduler/cache contamine fortement à cette
échelle.

**Mitigation** :
* sample_size 100 + criterion auto-tune iters/sample pour totaliser
  ~5-10 µs/sample → ~10 k iters/sample → CI très étroit.
* Cibles SLO formulées en p95 *avec marge ×3* (50 % de la cible
  est la mesure attendue).

---

## 9. Plan jour-par-jour (3 j wallclock)

### D3.1 (½ j) — Helpers et squelette

* Créer `compute_pipeline.rs` avec imports, `BLAS_SUFFIX`,
  `Fixture` const, `build_chain_inputs`, helpers `xorshift_vec`,
  `transpose_2d`, `build_one_hot_anchors`.
* Ajout `[[bench]] name = "compute_pipeline"` dans
  `efe-core/Cargo.toml`.
* `cargo build -p efe-core --benches` propre.

**DoD** :
- [ ] Squelette compile sans warning.
- [ ] `BLAS_SUFFIX` correctement gated.

### D3.2 (1 j) — Groupe 1 : `compute_pipeline_chain`

* Implémenter le chain bench × 3 fixtures.
* Vérifier que les outputs sont *finis* (pas de NaN/Inf) sur les
  3 fixtures.
* Faire passer `cargo bench --bench compute_pipeline -- --quick`
  sur small + medium.

**DoD** :
- [ ] Chain bench produit des mesures stables.
- [ ] No NaN/Inf dans les outputs (smoke test inline).
- [ ] Wall time `--quick` < 30 s.

### D3.3 (½ j) — Groupe 2 : `compute_pipeline_per_phase`

* Implémenter les 6 sub-benches × 3 fixtures = 18 sub-benches.
* Vérifier que la somme per_phase ≈ chain à ±15 %.

**DoD** :
- [ ] Per-phase running clean.
- [ ] Additivité validée à ±15 %.

### D3.4 (½ j) — Groupes 3 & 4 : throughput sweep + BLAS summary

* `compute_pipeline_throughput` : sweep n_agents ∈ {8…512}.
* `compute_pipeline_blas_speedup_summary` (cfg-gated).

**DoD** :
- [ ] Sweep produit une courbe O(N²) lisible.
- [ ] BLAS summary disponible avec `--features accelerate`.

### D3.5 (½ j) — Tests anti-régression + tooling

* `crates/efe-core/tests/compute_pipeline_realism.rs` (3 tests).
* `parse_criterion_report.py` : extension WANTED + 6 SLO `D3-*`.
* `bench_runner.sh` : ajout du run.
* `BENCHMARKS.md` § 5 « Compute pipeline (D3) ».
* `slo.md` § 6 « SLO bench compute pipeline ».
* `bench_baselines/v0.1.3/baseline.json` placeholder.

**DoD** :
- [ ] 3 tests réussissent.
- [ ] CI workflow inclut le path `efe-core/benches/compute_pipeline.rs`.
- [ ] Baseline placeholder valide JSON.

---

## 10. Calibration & nouveaux SLO `D3-*`

Référence : Apple M3 Pro, 11 cores, 36 GB, rustc 1.85.0.

### 10.1 Cibles SLO sans BLAS (`_naive` suffix)

| ID | Nom | Bench key | Cible p95 |
|----|-----|-----------|----------:|
| `SLO-D3-1` | Chain small (naïve) | `compute_pipeline_chain_naive::small` | ≤ **200 µs** |
| `SLO-D3-2` | Chain medium (naïve) | `compute_pipeline_chain_naive::medium` | ≤ **500 µs** |
| `SLO-D3-3` | Chain large (naïve) | `compute_pipeline_chain_naive::large` | ≤ **20 ms** |

### 10.2 Cibles SLO avec BLAS (`_blas` suffix)

| ID | Nom | Bench key | Cible p95 |
|----|-----|-----------|----------:|
| `SLO-D3-1B` | Chain small (BLAS) | `compute_pipeline_chain_blas::small` | ≤ **200 µs** *(no dispatch on small)* |
| `SLO-D3-2B` | Chain medium (BLAS) | `compute_pipeline_chain_blas::medium` | ≤ **250 µs** |
| `SLO-D3-3B` | Chain large (BLAS) | `compute_pipeline_chain_blas::large` | ≤ **5 ms** |

**Ratio attendu** : SLO-D3-2 / SLO-D3-2B = 2.0× ; SLO-D3-3 /
SLO-D3-3B = 4.0×. CI ne *gate* pas sur ce ratio (c'est une
métrique d'observation), mais BENCHMARKS.md le documente.

### 10.3 Coefficient CI

`bench.yml` applique automatiquement ×1.5 (matche D1/D2).

### 10.4 Per-phase et throughput — pas de SLO direct

Les groupes `compute_pipeline_per_phase` et `compute_pipeline_throughput`
sont mesurés et publiés mais **ne gate pas la PR**. Raison : leur
sample size plus faible (50 vs 100) rend la variance > seuil de
régression utile (>15%). Ils servent au diagnostic.

---

## 11. Anti-régression & realism check

**Fichier** : `crates/efe-core/tests/compute_pipeline_realism.rs`
(nouveau)

### Test 1 — `chain_produces_finite_outputs_on_all_fixtures`

```rust
#[test]
fn chain_produces_finite_outputs_on_all_fixtures() {
    let backend = CpuBackend::new();
    for fixture in FIXTURES {
        let inputs = build_chain_inputs(fixture);
        let scores     = backend.matmul(&inputs.q, &inputs.k_t).unwrap();
        let attention  = backend.softmax(&scores, -1).unwrap();
        let aggregated = backend.matmul(&attention, &inputs.q).unwrap();
        let projected  = backend.cone_project(&aggregated, &inputs.anchors, inputs.max_dist).unwrap();
        let entropy    = backend.shannon_entropy(&attention, -1).unwrap();
        let stats      = backend.std_dev(&projected, 0).unwrap();
        for (name, t) in [
            ("scores", &scores), ("attention", &attention),
            ("aggregated", &aggregated), ("projected", &projected),
            ("entropy", &entropy), ("stats", &stats),
        ] {
            assert!(t.data().iter().all(|x| x.is_finite()),
                "[{}] {} contains NaN/Inf", fixture.name, name);
        }
    }
}
```

### Test 2 — `chain_shapes_match_expected`

Verrouille les shapes de chaque output : si quelqu'un casse la
sémantique de `cone_project` pour retourner un (N,) au lieu de
(N, D), le bench mesure la mauvaise chose et ce test trip.

### Test 3 — `attention_rows_sum_to_one`

Sanity numérique : après softmax(scores, axis=-1), chaque row de
`attention` somme à 1.0 ± 1e-5. Si ce test trip, le bench mesure
une chaîne avec un softmax cassé — chiffres invalides.

---

## 12. Intégration CI + parser + baseline v0.1.3

### 12.1 `parse_criterion_report.py`

Étendre `WANTED_PREFIXES` :
```python
WANTED_PREFIXES = (
    # Sprint C, D1, D2 (existant)
    ...
    # Sprint D3
    "compute_pipeline_chain_",
    "compute_pipeline_per_phase_",
    "compute_pipeline_throughput_",
    "compute_pipeline_blas_speedup_",
)
```

Ajouter à `SLO_DEFINITIONS` les 6 SLO `D3-*` listés en §10.

### 12.2 `bench_runner.sh`

Dans la branche `[ "$ONLY" != "real" ]`, ajouter :
```bash
echo "==> Running compute_pipeline bench (Sprint D3, compute chain)"
$CARGO bench -p efe-core --bench compute_pipeline $CORE_FEATURES_FLAG -- \
    --output-format bencher $CRITERION_FLAGS \
    > "target/bench-compute-pipeline.txt" 2>&1 \
    || echo "WARN: compute_pipeline returned non-zero (continuing)"
```

`$CORE_FEATURES_FLAG` propage `accelerate`/`openblas` si l'opérateur
les demande via `BLAS_BACKEND=…`.

### 12.3 `.github/workflows/bench.yml`

Aucun ajout au path filter (`crates/efe-core/benches/**` couvre).
Étendre l'upload :
```yaml
path: |
  ...
  target/bench-compute-pipeline.txt
```

### 12.4 `bench_baselines/v0.1.3/baseline.json`

Placeholder identique au pattern v0.1.1/v0.1.2 : `results: {}`,
6 SLO `skipped: true`, _note explicatif. Première vraie baseline
post-merge sur `main`.

### 12.5 `BENCHMARKS.md` — nouvelle section § 5

Insérer après § 4 (Memory) et avant § 6 (Micro-benchmarks Sprint
2-4) :

```markdown
## 5. Compute pipeline (Sprint D3, v0.1.3)

Mesures de la **chaîne réelle** des 6 ops compute qu'une itération
de `MvpCpuFusion::run_round_sync` exécute. Lecture clé :

* La somme `compute_pipeline_chain_*::small` ≈ une fraction
  significative de `real_e2e_latency_naive::small` (D1) — D3
  expose la part compute pure, hors orchestration (D1) et
  hors mémoire (D2).
* Le ratio `_naive` / `_blas` quantifie le gain matériel BLAS sur
  les matmuls 1+3.

| Régime | Bench key | médian (M3 Pro) | SLO |
|--------|-----------|----------------:|----:|
| Chain small naïve | `compute_pipeline_chain_naive::small` | ~30 µs | 200 µs (×6 marge) |
| Chain medium naïve | `compute_pipeline_chain_naive::medium` | ~190 µs | 500 µs (×2.5) |
| Chain medium BLAS | `compute_pipeline_chain_blas::medium` | ~80 µs | 250 µs |
| Chain large naïve | `compute_pipeline_chain_naive::large` | ~10 ms | 20 ms |
| Chain large BLAS | `compute_pipeline_chain_blas::large` | ~3 ms | 5 ms |
```

### 12.6 `docs/slo.md` — nouvelle section § 6

Identique à D1 / D2 : 6 SLO `D3-*` listés en table + protocole
anti-régression (3 tests `compute_pipeline_realism.rs`).

---

## 13. Critères d'acceptation (DoD)

### 13.1 Code & build

- [ ] `crates/efe-core/benches/compute_pipeline.rs` créé, ~600 LOC.
- [ ] `crates/efe-core/Cargo.toml` ajout `[[bench]] name = "compute_pipeline"`.
- [ ] `cargo build -p efe-core --benches --release` clean.
- [ ] `cargo build -p efe-core --benches --release --features accelerate` clean
  (sur macOS).
- [ ] `cargo clippy -p efe-core --benches -- -D warnings` clean.

### 13.2 Tests

- [ ] `crates/efe-core/tests/compute_pipeline_realism.rs` créé,
      3 tests.
- [ ] `cargo test -p efe-core --test compute_pipeline_realism` →
      3/3 pass.
- [ ] `cargo test -p efe-core` (suite complète, 1 428+ tests)
      → tous pass.

### 13.3 Bench actually runs

- [ ] `cargo bench -p efe-core --bench compute_pipeline -- --quick`
      complète sans erreur.
- [ ] 4 groupes criterion produisent des mesures sur les 3 fixtures.
- [ ] Aucune mesure NaN / +inf / 0 (tests anti-régression
      l'attestent).

### 13.4 Tooling

- [ ] `scripts/bench_runner.sh` lance le nouveau bench.
- [ ] `scripts/parse_criterion_report.py` reconnaît
      `compute_pipeline_*`.
- [ ] `bench_baselines/v0.1.3/baseline.json` placeholder créé.
- [ ] `.github/workflows/bench.yml` upload artefact étendu.

### 13.5 Documentation

- [ ] `BENCHMARKS.md` § 5 « Compute pipeline (D3) » ajoutée.
- [ ] `docs/slo.md` § 6 « SLO D3 compute pipeline » ajoutée.
- [ ] `docs/bench/d3_compute_pipeline_architecture.md` (ce
      document) référencé en intro de `BENCHMARKS.md`.
- [ ] Tous les commentaires bench rationalisent leur sample size,
      cible SLO, et risque de mesure.

### 13.6 Validation finale

- [ ] Mesures cohérentes avec § 10 (×3 max d'écart vs cible).
- [ ] `compute_pipeline_chain_naive::medium` < 500 µs sur M3 Pro
      (cible primaire).
- [ ] Avec `--features accelerate` : `*_blas::medium` < 250 µs.
- [ ] Per-phase sum ≈ chain ± 15 % (additivité validée).

---

## 14. Out of scope & roadmap follow-ups

### 14.1 Hors-scope D3

* **β-mix et clip** : ~6 % du round time, pas d'op
  `ComputeBackend` directe. Couverts implicitement par D1.
* **`scaled_dot_product_attention` (fused)** : on bench les ops
  séparées pour exposer le data flow ; la version fused (Q+K+V→out
  en un seul appel) est mesurée par `compute_sprint3.rs`.
* **`matmul_fused_gelu`** : pas dans le chemin fusion. Mesuré par
  `compute_sprint4.rs`.
* **Heap profiling** : D3.5 follow-up `compute_pipeline_peak.rs`
  avec `peak_alloc` global allocator.
* **CUDA backend** : feature `cuda` gated, hors MVP. Phase 2.
* **Sweep `dim`** : v0.1.3 fixe `dim ∈ {128, 256}`. Sweep complet
  (`dim ∈ {64, 128, 256, 512, 1024}`) = follow-up D3.6.

### 14.2 Roadmap après D3

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D3.5** | `compute_pipeline_peak.rs` | Heap profiling de la chaîne (peak_alloc) |
| **D4** | `consensus_signals_pipeline.rs` | Bench des 6 signaux du `MultiCriteriaDetector` |
| **D5** | `genesis_pipeline.rs` | Bench du chemin meta+router+cardinality+spawner |
| **D6** | `dissolution_pipeline.rs` | Bench centroid extraction + alignment per-anchor |

À l'issue de **D3 + D4 + D5 + D6**, on aura **100 % de couverture
bench** des chemins prod, attribuables phase par phase.

### 14.3 Métrique de succès post-D3

À 30 jours après merge D3, on doit pouvoir affirmer :

> *« Toute régression > 50 % sur la chaîne compute MVP produit
>  un fail SLO bench-niveau dans les 24 h après le merge, avec
>  une attribution exacte à `matmul`, `softmax`, `cone_project`,
>  `shannon_entropy`, ou `std_dev`. Le ratio BLAS speedup est
>  publié à chaque release. »*

Si cette affirmation tient, D3 a rempli sa mission.

---

## Annexe A — Schéma de séquence : la chaîne D3

```
  ┌─────────────────┐  Q (N,D)
  │ build_inputs    │──────────────┐
  │ (xorshift, seed)│              ▼
  └─────────────────┘    ┌──────────────────┐
        │ K_T (D,N)      │ Phase 1 — matmul │ ⟶ scores (N,N)
        └───────────────►│      Q × K_T     │
                         └──────────┬───────┘
                                    ▼
                         ┌──────────────────┐
                         │ Phase 2 — softmax│ ⟶ attention (N,N)
                         │  axis = -1       │
                         └──────────┬───────┘
                                    ▼
        Q reused as V    ┌──────────────────┐
        ────────────────►│ Phase 3 — matmul │ ⟶ aggregated (N,D)
                         │  attn × Q        │
                         └──────────┬───────┘
                                    ▼
        anchors (A,D)    ┌──────────────────┐
        ────────────────►│ Phase 4 — cone   │ ⟶ projected (N,D)
                         │     project       │
                         └──────────┬───────┘
                                    ▼
                         ┌──────────────────┐
                attention│ Phase 5 — entropy│ ⟶ entropy (N,)
                ─────────┤  shannon, axis=-1│
                         └──────────┬───────┘
                                    │ + projected (N,D)
                                    ▼
                         ┌──────────────────┐
                         │ Phase 6 — std_dev│ ⟶ stats (D,)
                         │     axis = 0     │
                         └──────────────────┘
```

## Annexe B — Coûts par phase et fixture (M3 Pro estimés)

### B.1 Naïve

| Fixture | P1 matmul | P2 softmax | P3 matmul | P4 cone | P5 entropy | P6 std_dev | TOTAL |
|---------|----------:|-----------:|----------:|--------:|-----------:|-----------:|------:|
| small (8/128/4) | 1 µs | 1 µs | 1 µs | 5 µs | 1 µs | 2 µs | **~12 µs** |
| medium (32/256/8) | 70 µs | 5 µs | 70 µs | 30 µs | 5 µs | 10 µs | **~190 µs** |
| large (128/256/16) | 4 ms | 50 µs | 4 ms | 1 ms | 50 µs | 100 µs | **~9.2 ms** |

### B.2 Avec Apple Accelerate

| Fixture | P1 matmul | P2 softmax | P3 matmul | P4 cone | P5 entropy | P6 std_dev | TOTAL |
|---------|----------:|-----------:|----------:|--------:|-----------:|-----------:|------:|
| small | 1 µs (no dispatch) | 1 µs | 1 µs | 5 µs | 1 µs | 2 µs | ~12 µs |
| medium | 5 µs | 5 µs | 5 µs | 30 µs | 5 µs | 10 µs | **~60 µs** |
| large | 100 µs | 50 µs | 100 µs | 1 ms | 50 µs | 100 µs | **~1.4 ms** |

**Gain BLAS attendu** : ×2-3 sur medium ; ×6-7 sur large. Domine
sur les matmuls (P1, P3) ; les autres ops restent au coût naïf.

## Annexe C — Diagramme dépendance D3

```
                  ┌────────────────────────┐
                  │ benches/compute_pipeline.rs │
                  └───────────┬────────────┘
                              │ uses
            ┌─────────────────┼──────────────────┐
            ▼                 ▼                  ▼
    ┌──────────────┐  ┌──────────────────┐  ┌─────────────────┐
    │ benches/     │  │ efe_core::compute│  │ criterion 0.5    │
    │ common/mod.rs│  │  ::ComputeBackend│  │  (no async here) │
    │ (xorshift)   │  │  ::CpuBackend    │  └─────────────────┘
    └──────────────┘  │  ::Tensor<f32>   │
                      └────────┬─────────┘
                               │
                ┌──────────────┼──────────────┐
                ▼              ▼              ▼
         ┌─────────────┐ ┌────────────┐ ┌────────────┐
         │ matmul      │ │ softmax    │ │ cone_project│
         │ (naive/BLAS)│ │ shannon_e  │ │ std_dev    │
         └─────────────┘ └────────────┘ └────────────┘
```

**Aucune dépendance vers `efe-server`, `efe-telemetry`, `tokio`,
`peak_alloc`** — D3 est purement compute synchrone.

---

## Annexe D — Checklist exécution architect → impl

```
□ D3.1.1  Squelette + imports + BLAS_SUFFIX cfg
□ D3.1.2  FIXTURES const + Fixture struct
□ D3.1.3  build_chain_inputs (xorshift, transpose, anchors)
□ D3.1.4  Cargo.toml [[bench]] entry

□ D3.2.1  bench_compute_pipeline_chain × 3 fixtures
□ D3.2.2  Validate finite outputs (smoke print)
□ D3.2.3  cargo bench --quick run propre

□ D3.3.1  bench_compute_pipeline_per_phase × 6 phases × 3 fixtures
□ D3.3.2  Mental check: per-phase sum ≈ chain ±15%

□ D3.4.1  bench_compute_pipeline_throughput sweep n_agents
□ D3.4.2  bench_compute_pipeline_blas_speedup_summary cfg-gated

□ D3.5.1  tests/compute_pipeline_realism.rs (3 tests)
□ D3.5.2  parse_criterion_report.py extension WANTED + SLO
□ D3.5.3  bench_runner.sh ajout
□ D3.5.4  BENCHMARKS.md § 5 + slo.md § 6
□ D3.5.5  bench_baselines/v0.1.3/baseline.json placeholder
□ D3.5.6  .github/workflows/bench.yml upload path

□ Validation finale
  □ cargo test -p efe-core (full)
  □ cargo clippy -p efe-core --benches -- -D warnings
  □ cargo bench --bench compute_pipeline -- --quick (naïve)
  □ cargo bench --bench compute_pipeline --features accelerate -- --quick (BLAS, sur macOS)
  □ Vérification ordres de grandeur §10 ±3×
  □ Per-phase sum ≈ chain ±15% sur les 3 fixtures
```

---

**Fin de l'architecture D3.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à §6 pour le détail de chaque
groupe et §8 pour les pièges à éviter.

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue technique).
> - [ ] Owner `efe_core::compute` (revue ComputeBackend usage).
> - [ ] Lead infra (revue intégration CI + baselines).
