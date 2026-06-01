# M5 — Fusion Kernels
## Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Module** : `efe_core::fusion` (code) + `efe_kernels::fusion` (kernels hot-path)
**Sprints ciblés** : Sprint 4 (MVP CPU) → Sprint 5 (optimisation + stats kernel GPU)
**Dépend de** : M1 (Core Runtime), M2 (Memory Pool), M3 (Genesis Engine), M4 (ELS)
**Consommé par** : M6 (Consensus Detector), M1 controller, M11 (Telemetry)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP — ce qui est IN et OUT](#2-scope-mvp)
3. [Théorie : swarm attention](#3-théorie--swarm-attention)
4. [Vue d'ensemble architecturale](#4-vue-densemble-architecturale)
5. [Traits et abstractions](#5-traits-et-abstractions)
6. [Structures de données](#6-structures-de-données)
7. [Flux d'un round de fusion](#7-flux-dun-round-de-fusion)
8. [Algorithmes détaillés](#8-algorithmes-détaillés)
9. [Gestion des erreurs et anomalies](#9-gestion-des-erreurs-et-anomalies)
10. [Performance et budget temporel](#10-performance-et-budget-temporel)
11. [Interface publique](#11-interface-publique)
12. [Stratégie de test](#12-stratégie-de-test)
13. [Dépendances inter-modules](#13-dépendances-inter-modules)
14. [Plan de sprint M5](#14-plan-de-sprint-m5)
15. [Critères d'acceptation](#15-critères-dacceptation)
16. [Risques et mitigations](#16-risques-et-mitigations)
17. [Annexes](#17-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M5 est le **cœur computationnel du round d'inférence** de l'EFE. À chaque round
`r`, il applique un opérateur de fusion `F: ℝ^{N×D} → ℝ^{N×D}` sur le tenseur
latent de la population d'agents, conservant :

- **Dimensionalité** : le nombre d'agents actifs `N_active` et la dimension
  latente `D` ne changent pas durant la fusion.
- **Masque** : les agents inactifs voient leurs lignes laissées inchangées
  (lecture seule — le kernel ne les considère ni comme sources ni comme cibles).
- **Alignement** : après fusion, chaque ligne est soft-projetée vers le cône
  d'anchors défini dans l'ELS, avec une force décroissante `α_r` sur les rounds.

En une phrase : **M5 fait "parler" les agents entre eux dans l'espace latent
partagé**, en remplaçant le texte par un produit scalaire pondéré.

### 1.2 Contrainte d'abstraction : pas d'I/O hors ELS

M5 **ne lit et n'écrit que via les vues M4** — aucun accès au réseau, au
filesystem, à un LLM externe, ni à la mémoire hors-arena. Cela garantit :

1. **Reproductibilité** : pour un même `SwarmId`, un même `FusionConfig` et un
   même état initial de l'ELS, `N` rounds produisent exactement le même tenseur
   final en CPU (dépendance à `target-cpu=native` documentée).
2. **Profilabilité** : le temps mur d'un round = temps CPU + 0. Pas de cause
   externe de latence.
3. **Testabilité** : tests unitaires purs, pas de doubles pour I/O.

### 1.3 Position dans le pipeline EFE

```
          ┌─────────────────────────────────────────────────────────┐
          │                    Controller (M1)                       │
          └───┬──────────────┬──────────────┬──────────────┬─────────┘
              │              │              │              │
          spawn()        fuse()        detect()        collapse()
              │              │              │              │
              ▼              ▼              ▼              ▼
          ┌───────┐     ┌──────────┐   ┌─────────┐    ┌────────┐
          │  M3   │──▶  │   M5     │──▶│   M6    │──▶ │   M7   │
          │Genesis│     │  Fusion  │   │Consensus│    │Collapse│
          └───┬───┘     └────┬─────┘   └────┬────┘    └────────┘
              │              │              │
              ▼              ▼              ▼
          ┌─────────────────────────────────┐
          │      M4 — ELS (shared state)    │
          │   latent, mask, anchors, emb    │
          └─────────────────────────────────┘
                         ▲
                         │ split_mut()
                         │
                   ┌─────┴─────┐
                   │   M2      │
                   │  Memory   │
                   └───────────┘
```

### 1.4 Garanties offertes aux modules avals

1. **Monotonie de variance** : dans des conditions nominales (pas d'anomalie),
   `variance(r+1) ≤ variance(r) + ε_drift` avec `ε_drift = 0.01 * variance(0)`.
   Garantit que la fusion converge, critère nécessaire à M6.
2. **Préservation de la norme** : `|| h_i(r+1) ||_2 ∈ [0.5 * E[|| h ||], 2 * E[|| h ||]]`
   — empêche l'explosion / annulation.
3. **Bornage** : toute coordonnée `|h_i[j]| ≤ CLIP_MAX = 32.0` après chaque round.
   Dépasser cette valeur déclenche `EfeError::NumericalAnomaly` propagée.
4. **Déterminisme binaire** : sur HostBackend + `-C target-cpu=native` désactivé
   en CI, deux runs consécutifs du même round produisent des bytes bf16
   identiques.

---

## 2. Scope MVP

### 2.1 IN

| Fonctionnalité | Justification MVP |
|---|---|
| Swarm attention single-head | Suffit pour la thèse — simple, bornée, testable. |
| Softmax stabilisée (max-shift) | Nécessaire pour éviter l'overflow à `D ≥ 512`. |
| Agrégation pondérée (weighted sum) | Formulation canonique de l'attention. |
| Soft-projection sur cône alignment | Permet de rejoindre la politique d'alignement sans re-upload. |
| Trait `FusionStrategy` (DI) | Permet swap CPU/CUDA sans toucher M1. |
| Backend CPU (Rayon optionnel) | MVP tourne sur host. |
| `FusionStats` émission per-round | Consommé par M6 et M11. |
| Détection NaN/Inf avec court-circuit | Garantit que le controller peut fallback. |
| Anomaly replay — buffer des N derniers latents | Permet à M7 de rollback. |

### 2.2 OUT (reporté Sprint 5+)

| Fonctionnalité | Raison du report | Sprint ciblé |
|---|---|---|
| Multi-head attention | Complexité non justifiée pour MVP. | Sprint 7 |
| CUDA kernel fusionné | Nécessite Sprint 3 (wrapper CUDA stable). | Sprint 6 |
| Attention sparsifiée (top-k) | Optimisation perf, pas un blocker. | Sprint 8 |
| Cross-swarm attention | Dépend de M8 (multi-swarm coordination). | Sprint 9 |
| Quantification int8 | Nécessite calibration, hors scope MVP. | v2 |
| LoRA-style weight deltas | Pas de weights dans le MVP (attention paramétrique zero-param). | v2 |
| Différentiation automatique | M5 MVP est forward-only. | v2 |

### 2.3 Principe "zero-param attention"

**Le MVP implémente une attention sans paramètres entraînables** :

```
α_ij = softmax_j( <h_i, h_j> / sqrt(D) )
h'_i = Σ_j α_ij * h_j
```

Pas de matrices `Wq, Wk, Wv, Wo`. Raisons :

1. Pas d'entraînement nécessaire — on part d'un latent déjà sensé (M3 fournit
   des agents template-conditionnés).
2. Chaque agent est déjà spécialisé par template ; la fusion agrège leurs
   perspectives sans les re-projeter.
3. Teste la thèse centrale : **la géométrie seule suffit-elle ?**

Si l'ablation démontre un gap, Sprint 5+ introduit une projection légère
(rang-réduit) en option derrière un feature flag.

---

## 3. Théorie : swarm attention

### 3.1 Formulation mathématique

Soit `H^(r) ∈ ℝ^{N×D}` le latent au round `r`, `m ∈ {0,1}^N` le masque actif,
`A ∈ ℝ^{K×D}` les anchors, `p ∈ ℝ^D` l'embedding du problème. Un round de M5
produit `H^(r+1)` selon :

**Étape 1 — Score d'attention (non normalisé) :**

```
s_ij = (h_i · h_j) / √D      pour i, j actifs
s_ij = -∞                    si m_j = 0  (masqué)
```

**Étape 2 — Softmax stabilisée :**

```
μ_i = max_j s_ij
α_ij = exp(s_ij - μ_i) / Z_i    où Z_i = Σ_j exp(s_ij - μ_i)
```

**Étape 3 — Agrégation pondérée :**

```
h'_i = Σ_j α_ij * h_j
```

**Étape 4 — Influence de l'embedding du problème (ancrage doux) :**

```
h''_i = (1 - β_r) * h'_i + β_r * p
```
où `β_r = β_0 * exp(-λ_β * r)` est une force décroissante (le swarm
s'émancipe progressivement de la formulation initiale).

**Étape 5 — Projection sur le cône d'alignment :**

```
h_i^(r+1) = π_α(h''_i, A, α_r)
```
où `π_α` est l'opérateur `project_onto_anchors` de M4 (§ projection.rs), avec
`α_r = α_0 * (1 - r / R_max)` — force d'alignement qui diminue linéairement
(démarre serré, assouplit à mesure que la convergence approche).

**Étape 6 — Clipping anti-divergence :**

```
h_i^(r+1)[d] = clip(h_i^(r+1)[d], -CLIP_MAX, CLIP_MAX)
```

### 3.2 Complexité

| Étape | Flops | Mémoire scratch |
|---|---|---|
| Scores | `O(N² · D)` | `O(N²)` |
| Softmax | `O(N²)` | in-place |
| Agrégation | `O(N² · D)` | — (écrit dans latent) |
| β-mix | `O(N · D)` | — |
| Projection | `O(N · K · D)` | `O(N · K)` |
| Clip | `O(N · D)` | — |

**Domine : `O(N² · D)`** pour les scores + l'agrégation.

Budget numérique pour le MVP (`N=128`, `D=2048`) :
- Scores : 128² × 2048 = 33.5 Mflops
- Softmax : 16.4 Kflops (négligeable)
- Agrégation : 33.5 Mflops
- Total par round ≈ **67 Mflops**, négligeable pour 16 rounds (≈1 Gflops) sur
  CPU moderne (≈100 Gflops/s FP32 réaliste) → **10 ms théoriques**.

### 3.3 Propriétés mathématiques recherchées

1. **Contraction vers le centroïde des actifs** : `|| h_i(r+1) - μ || < || h_i(r) - μ ||`
   pour la plupart des `i` (propriété moyenne — la softmax ne garantit pas
   strictement mais la variance décroît).
2. **Invariance sous permutation** : l'ordre des agents ne change pas le
   résultat. Vérifié par test unitaire.
3. **Lipschitz borné** : petite perturbation d'un agent → perturbation bornée
   des autres. Évite les effets "chaotiques" de divergence.
4. **Fixe point d'équilibre** : un swarm où toutes les lignes sont égales à un
   même vecteur `h*` est un point fixe de la fusion (à la β-mix près).

### 3.4 Pourquoi pas d'attention classique multi-head à 3 projections ?

- **Mémoire** : multi-head × `Wq/Wk/Wv/Wo` ≈ 4 × D² weights → 4 × 2048² × 2
  bytes = **32 MB par head**. Multi-head = plusieurs × 32 MB. Hors budget
  HBM MVP (256 MB réservés pour l'ELS).
- **Entraînement** : introduire des poids nécessite un signal d'entraînement,
  c'est M5+ (reinforcement from task outcome), ou un modèle pré-entraîné
  importé — hors scope MVP.
- **Signal du swarm** : les agents portent déjà une diversité sémantique via
  les templates (`NaSem`, `NaLog`, etc.) — l'attention zero-param exploite
  cette diversité directement.

---

## 4. Vue d'ensemble architecturale

### 4.1 Disposition des crates

```
efe-core/
├── src/
│   ├── fusion/
│   │   ├── mod.rs               # Pub façade, FusionStrategy trait
│   │   ├── config.rs            # FusionConfig
│   │   ├── round.rs             # Boucle per-round (orchestration)
│   │   ├── stats.rs             # FusionStats (déjà en place M3) — extensions
│   │   ├── mvp_cpu.rs           # FusionStrategy impl CPU MVP
│   │   ├── replay.rs            # Ring buffer pour snapshot anti-divergence
│   │   └── validator.rs         # Pre/post-condition checks
│   └── fusion.rs                # re-export (compat façade existante)
efe-kernels/
├── src/
│   ├── fusion/
│   │   ├── mod.rs               # Re-exports
│   │   ├── attention.rs         # Kernel attention CPU (core math)
│   │   ├── softmax.rs           # Softmax stable (max-shift)
│   │   ├── aggregate.rs         # Agrégation pondérée
│   │   ├── clip.rs              # Clipping anti-divergence
│   │   └── simd.rs              # Helpers SIMD optionnels (feature=native)
```

**Séparation `efe-core` / `efe-kernels`** :
- `efe-core::fusion` : orchestration, API stable, types `pub`. Pas de math
  performance-critique.
- `efe-kernels::fusion` : math pure, `#[inline]` systématique, SIMD-friendly.
  Interface : `pub(crate)` du point de vue de `efe-core`, `pub` pour perm.
  swap CPU/CUDA backend.

### 4.2 Diagramme de classes (vue logique)

```
                  ┌──────────────────────────────┐
                  │     FusionStrategy (trait)   │
                  │  fn run(els, cfg) → Round    │
                  └────────────┬─────────────────┘
                               │ impl
             ┌─────────────────┼─────────────────┐
             │                 │                 │
     ┌───────▼───────┐ ┌───────▼───────┐ ┌───────▼────────┐
     │  MvpCpuFusion │ │ CudaFusion    │ │MockFusion      │
     │  (Sprint 4)   │ │  (Sprint 6)   │ │  (tests)       │
     └───────┬───────┘ └───────┬───────┘ └────────────────┘
             │ uses            │ uses
             ▼                 ▼
     ┌──────────────────────────────┐
     │     FusionKernel (trait)     │
     │ attention / softmax / …      │
     └────────────┬─────────────────┘
                  │ impl
       ┌──────────┼──────────┐
       ▼                     ▼
 ┌──────────┐         ┌──────────┐
 │ CpuKernel│         │CudaKernel│
 └──────────┘         └──────────┘
```

### 4.3 Pipeline de données d'un round

```
     ┌───────────────────────────────────────────────────────┐
     │                    M5 — run_round()                    │
     └───────────────────────────────────────────────────────┘
                             │
           ┌─────────────────┼─────────────────┐
           ▼                 ▼                 ▼
     ┌──────────┐     ┌──────────┐      ┌──────────┐
     │ validate │     │  stats   │      │  split   │
     │ preconds │     │  (pre)   │      │  mut ELS │
     └────┬─────┘     └────┬─────┘      └────┬─────┘
          │                │                  │
          └────────────┬───┴──────────────────┘
                       ▼
                ┌──────────────┐
                │ attn scores  │  O(N²·D) read latent
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │  softmax     │  O(N²)   in-place
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │  aggregate   │  O(N²·D) write latent'
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │   β-mix p    │  O(N·D)
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │   project    │  O(N·K·D) via M4
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │    clip      │  O(N·D)
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │ stats (post) │  → FusionStats
                └──────┬───────┘
                       ▼
                ┌──────────────┐
                │  validate    │  post-conds
                └──────┬───────┘
                       ▼
                    return
```

---

## 5. Traits et abstractions

### 5.1 `FusionStrategy` — façade de haut niveau

```rust
/// Execute one or more fusion rounds over a swarm's ELS.
///
/// Implementations:
///   - `MvpCpuFusion`  — pure Rust CPU, MVP
///   - `CudaFusion`    — GPU, Sprint 6+
///   - `MockFusion`    — deterministic no-op for tests
///
/// # Contract
///
/// - `run_round` must be *idempotent on observable state*: calling it on an
///   ELS with identical bytes must produce identical new bytes.
/// - Exclusive access: `els: &mut EpistemicLatentSpace` is required.
/// - Non-blocking: must never hold a lock beyond `fn` scope.
#[async_trait::async_trait]
pub trait FusionStrategy: Send + Sync + std::fmt::Debug {
    /// Execute exactly one round. Returns the round outcome.
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        cfg: &FusionConfig,
        round: u32,
    ) -> Result<FusionRound>;

    /// Human-readable backend name (for telemetry).
    fn backend_name(&self) -> &'static str;

    /// Whether the implementation supports `replay()`.
    fn supports_replay(&self) -> bool { false }
}
```

### 5.2 `FusionKernel` — interface d'exécution bas niveau

```rust
/// Pure math kernels. Take borrowed views and mutate in place.
/// No awareness of rounds, config, arenas — just tensors.
///
/// Used internally by `FusionStrategy` implementations. The split
/// allows Sprint 6's `CudaFusion` to plug in a GPU `FusionKernel`
/// without rewriting the orchestration layer.
pub trait FusionKernel: Send + Sync {
    /// In-place fusion step, consumes views from `ElsSplitMut`.
    ///
    /// # Invariants entering the call
    /// - `split.latent.dim() == split.anchors.dim() == split.problem_embedding.dim()`
    /// - `split.mask.active_count() > 0`
    ///
    /// # Invariants on return
    /// - `split.latent` has been replaced by the post-fusion tensor.
    /// - Inactive rows are unchanged.
    /// - No NaN/Inf present (or `NumericalAnomaly` returned).
    fn step(&self, split: &mut ElsSplitMut<'_>, params: &KernelParams) -> Result<StepMetrics>;

    fn name(&self) -> &'static str;
}
```

### 5.3 `FusionStrategy::replay()` — extension optionnelle

```rust
/// Undo the last applied round by swapping latent bytes with a snapshot.
/// Strategies may opt out by returning `NotImplemented`.
pub trait ReplayableFusion: FusionStrategy {
    /// Snapshot the current latent. Must be called *before* run_round().
    fn snapshot_pre(&self, els: &EpistemicLatentSpace, arena: &Arena) -> Result<Snapshot>;

    /// Restore ELS latent from a snapshot.
    fn restore(&self, els: &mut EpistemicLatentSpace, arena: &Arena, snap: &Snapshot) -> Result<()>;
}
```

M5 MVP implémente `ReplayableFusion` via un ring-buffer à N=4 snapshots dans
une arena dédiée.

### 5.4 Choix DI : `Arc<dyn FusionStrategy>` dans le controller

```rust
pub struct SwarmController {
    genesis: Arc<dyn GenesisStrategy>,
    fusion:  Arc<dyn FusionStrategy>,     // ← injecté ici
    consensus: Arc<dyn ConsensusDetector>,
    // …
}
```

Le test unitaire du controller passe un `MockFusion` ; le binaire de prod
passe un `MvpCpuFusion`.

---

## 6. Structures de données

### 6.1 `FusionConfig`

```rust
/// Configuration for one fusion campaign (N rounds).
///
/// All values are set by the controller (M1) from the SwarmSpec or a
/// builder pattern. Validated eagerly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FusionConfig {
    // ---- Schedule ----
    /// Maximum number of rounds. Guard against non-convergence.
    pub max_rounds: u32,              // default: 16

    /// Hard wallclock budget per round (ms). Kernel honors deadline.
    pub round_deadline_ms: u32,       // default: 30

    // ---- Attention ----
    /// Temperature scale for softmax. Higher = softer distribution.
    pub temperature: f32,              // default: 1.0

    /// When true, mask self-attention (α_ii = 0). Reduces "self-loop" bias.
    pub mask_self_attention: bool,     // default: false (self-loop allowed)

    // ---- Alignment ----
    /// Initial alignment force α_0.
    pub alignment_strength_init: f32,  // default: 0.5

    /// Minimum alignment force (asymptote, round R_max).
    pub alignment_strength_floor: f32, // default: 0.05

    // ---- Problem anchor ----
    /// Initial β_0 weight on problem embedding.
    pub problem_anchor_init: f32,      // default: 0.2

    /// Exponential decay rate of β across rounds.
    pub problem_anchor_decay: f32,     // default: 0.15

    // ---- Safety ----
    /// Clip threshold (coordinate-wise).
    pub clip_max: f32,                 // default: 32.0

    /// When true, run post-round validator. Off in production hot-path.
    pub validate_post: bool,           // default: cfg!(debug_assertions)

    /// Enable replay buffer (required for M7 rollback). Adds ~N*D*2 bytes.
    pub enable_replay: bool,           // default: true
}
```

### 6.2 `FusionRound` — résultat d'un round

```rust
/// Outcome of a single fusion round.
/// This is what `FusionStrategy::run_round` returns.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FusionRound {
    /// Round index (0-based).
    pub round: u32,

    /// Stats emitted to M6 and M11.
    pub stats: FusionStats,

    /// Per-kernel timing (μs).
    pub timing: RoundTiming,

    /// Backend that executed this round.
    pub backend: &'static str,

    /// Was the deadline honored?
    pub on_time: bool,

    /// Non-fatal issues encountered (clipped rows, anchor zero-weight, etc.).
    pub warnings: Vec<FusionWarning>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoundTiming {
    pub scores_us: u32,
    pub softmax_us: u32,
    pub aggregate_us: u32,
    pub beta_mix_us: u32,
    pub project_us: u32,
    pub clip_us: u32,
    pub stats_us: u32,
    pub total_us: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FusionWarning {
    ClippedCoordinates { count: u32, max_abs: f32 },
    AnchorAllZeroWeight,
    DeadlineExceeded { budget_us: u32, actual_us: u32 },
    HighVarianceDelta { pre: f32, post: f32 },
}
```

### 6.3 `FusionStats` — rappel + extensions

La struct `FusionStats` existe depuis M3 (utilisée par M6). M5 la remplit
entièrement :

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FusionStats {
    pub round: u32,
    pub centroid: Vec<f32>,            // length == D
    pub variance: f32,                 // population var over active rows
    pub gradient_norm: f32,            // sqrt(variance) MVP
    pub outlier_ratio: f32,            // > 2σ fraction
    pub confidence: f32,               // filled by M6 on convergence
    pub has_nan: bool,
    pub has_inf: bool,

    // Nouveaux champs M5 :
    pub mean_attention_entropy: f32,   // H̄ = (1/N) Σ_i H(α_i·)
    pub max_attention_entropy: f32,
    pub min_attention_entropy: f32,
    pub beta_applied: f32,             // β_r used this round
    pub alpha_applied: f32,            // α_r used this round
}
```

L'entropie d'attention est un signal précieux pour M6 :
- `H ≈ log(N)` : distribution uniforme, l'agent n'a pas "choisi" → pas encore
  de consensus.
- `H → 0` : distribution pointue, l'agent a polarisé vers 1-2 sources →
  attention déterministe, consensus émergeant.

### 6.4 `KernelParams` — params passés aux kernels bas niveau

```rust
pub struct KernelParams {
    pub temperature: f32,
    pub mask_self_attention: bool,
    pub beta: f32,
    pub alpha: f32,
    pub clip_max: f32,
}
```

Plat, `Copy`, pas de `Box`/`Vec`. Passé par valeur aux kernels.

### 6.5 `ReplayBuffer` — ring de snapshots

```rust
/// Ring buffer of latent snapshots for M7 rollback.
///
/// Allocates its own arena (`Arena::new(N * snap_size)`), separate from
/// the ELS arena so that `els.reset_arena()` does not invalidate snapshots.
pub struct ReplayBuffer {
    arena: Arena,
    capacity: u32,                     // default: 4
    snapshots: VecDeque<SnapshotSlot>,
}

struct SnapshotSlot {
    round: u32,
    latent_raw: RawAllocation,         // bf16, N*D*2 bytes
    stats: ElsStats,
}
```

API :
```rust
impl ReplayBuffer {
    pub fn new(capacity: u32, arena_size_bytes: u64, backend: Arc<dyn DeviceBackend>) -> Result<Self>;
    pub fn push(&mut self, round: u32, els: &EpistemicLatentSpace, arena: &Arena) -> Result<()>;
    pub fn restore(&self, round: u32, els: &mut EpistemicLatentSpace, arena: &Arena) -> Result<()>;
    pub fn len(&self) -> usize;
}
```

### 6.6 `StepMetrics` — métriques émises par un kernel

```rust
pub struct StepMetrics {
    pub scores_computed: u32,      // N² — N if self-masked
    pub mean_attention_entropy: f32,
    pub min_attention_entropy: f32,
    pub max_attention_entropy: f32,
    pub clipped_rows: u32,
    pub clipped_max_abs: f32,
    pub anchor_zero_weight_events: u32,
}
```

---

## 7. Flux d'un round de fusion

### 7.1 Séquence chronologique

```
T=0μs         — run_round() entry
T=~5μs        — FusionConfig validation (temperature > 0, etc.)
T=~10μs       — ELS preconditions (is_populated, n_active > 0)
T=~50μs       — replay_buffer.push(round, els) if enabled
T=~100μs      — split_mut(&arena) → ElsSplitMut
T=~150μs      — kernel.step(split, params)
                  ├ scores_to_buffer    (scratch = N² floats)
                  ├ softmax_in_place    (scratch reused)
                  ├ aggregate_into_new_latent
                  │    (needs second buffer = N*D bf16)
                  ├ beta_mix_problem_emb
                  ├ project_onto_anchors (calls M4)
                  └ clip_coordinates
T=~15000μs    — post-fusion stats (compute_stats_inner)
T=~16000μs    — post-conditions validation
T=~16100μs    — build FusionRound → return
```

### 7.2 Allocations du round

Le round **ne fait aucune allocation dans la hot-path après setup**. Les
scratch buffers nécessaires sont pré-alloués dans l'arena via `reserve_scratch()` :

- `scores` : `N * N * 4` bytes (f32, réutilisé entre rounds)
- `new_latent` : `N * D * 2` bytes (bf16, double-buffering)

Allocation totale per-swarm : `N² * 4 + N * D * 2` bytes.
Pour `N=128, D=2048` : 65 KB + 512 KB = **~580 KB** → rentre dans l'arena
existante de 4 MB/swarm.

### 7.3 Double-buffering du latent

Pour éviter `h_i = Σ_j α_ij * h_j` de se lire soi-même en cours d'écriture, on
double-buffer :

```rust
// Round r:
//   read_buffer  ← latent (current)
//   write_buffer ← scratch[new_latent]
//   for i in 0..N: write_buffer[i] = Σ α_ij * read_buffer[j]
//   swap(latent_ptr, scratch_new_latent_ptr)
```

Le swap se fait au niveau `RawAllocation` : on échange les pointeurs sans
copier. À la fin du round, l'ELS pointe vers l'ex-scratch, et l'ex-latent
devient le nouveau scratch pour le round suivant.

Contrat : le swap doit être **atomique** du point de vue des observateurs
externes (M11 qui lit via `split_mut` en read-only). Rust `&mut self` sur
l'ELS le garantit.

### 7.4 Intégration M4 — `split_mut` au bon moment

```rust
pub async fn run_round(
    &self,
    els: &mut EpistemicLatentSpace,
    arena: &Arena,
    cfg: &FusionConfig,
    round: u32,
) -> Result<FusionRound> {
    self.validate_config(cfg)?;
    if cfg.enable_replay { self.replay.push(round, els, arena)?; }

    let pre_stats = els.snapshot(arena)?;
    let mut split = els.split_mut(arena)?;   // ← takes &mut els exclusive

    let params = self.params_for_round(cfg, round);
    let metrics = self.kernel.step(&mut split, &params)?;

    drop(split);    // release mutable borrow ← required before post-stats
    let post_stats = els.snapshot(arena)?;

    let fusion_stats = self.build_stats(round, &pre_stats, &post_stats, &metrics, &params);
    let warnings = self.collect_warnings(&metrics, &params);

    Ok(FusionRound {
        round,
        stats: fusion_stats,
        timing: self.timing.take(),
        backend: self.backend_name(),
        on_time: self.deadline_honored(cfg),
        warnings,
    })
}
```

---

## 8. Algorithmes détaillés

### 8.1 Compute attention scores

```rust
/// Fills `scores[i*N + j] = (h_i · h_j) / sqrt(D)` for i,j in active.
/// Inactive rows → -INFINITY so softmax assigns them weight 0.
///
/// Complexity: O(N² D)
///
/// # Numerical notes
/// - Accumulator in f32 even though latent is bf16.
/// - Division by sqrt(D) done after the dot to preserve precision.
/// - self-masking: if cfg.mask_self_attention, s_ii = -INFINITY
pub(crate) fn compute_scores(
    latent: &LatentView<'_>,
    mask: &MaskView<'_>,
    scores: &mut [f32],
    params: &KernelParams,
) {
    let n = latent.n_agents() as usize;
    let d = latent.dim() as usize;
    let inv_sqrt_d = 1.0 / (d as f32).sqrt();
    debug_assert_eq!(scores.len(), n * n);

    for i in 0..n {
        if !mask.is_active(i as u32) {
            // Fill row i with -inf; this agent doesn't attend at all.
            scores[i * n..(i + 1) * n].fill(f32::NEG_INFINITY);
            continue;
        }
        let h_i = latent.row(i as u32).expect("checked in loop");
        for j in 0..n {
            if !mask.is_active(j as u32) {
                scores[i * n + j] = f32::NEG_INFINITY;
                continue;
            }
            if params.mask_self_attention && i == j {
                scores[i * n + j] = f32::NEG_INFINITY;
                continue;
            }
            let h_j = latent.row(j as u32).expect("checked");
            let mut acc = 0.0f32;
            for (x, y) in h_i.iter().zip(h_j.iter()) {
                acc += x.to_f32() * y.to_f32();
            }
            scores[i * n + j] = acc * inv_sqrt_d / params.temperature;
        }
    }
}
```

**Optimisations Sprint 5** :
- Parallélisation Rayon : `scores.par_chunks_exact_mut(n)` sur `i`.
- SIMD via `std::simd` feature flag — remplacer la boucle produit scalaire
  par `f32x8` ou `f32x16` suivant la cible.
- Cache-aware tiling : traiter les `(i, j)` par blocs `B×B=32×32` pour tenir
  dans L1/L2.

### 8.2 Softmax stabilisée

```rust
/// In-place row-wise softmax on a N×N matrix.
///
/// Row i:
///   μ = max(s_ij)
///   exp_ij = exp(s_ij - μ)
///   Σ = Σ_j exp_ij
///   α_ij = exp_ij / Σ
///
/// After this, each row sums to 1 exactly, except:
/// - If all entries are -inf (no active targets), row is all-zero
///   (signals "no valid attention"). Caller handles this as an
///   identity row (h'_i := h_i).
///
/// # Numerical notes
/// - Max-shift protects against overflow at any f32-finite input.
/// - Kahan sum optional (not default — the loss of precision is under
///   1 ulp for reasonable D).
pub(crate) fn softmax_in_place_rows(scores: &mut [f32], n: usize) {
    debug_assert_eq!(scores.len(), n * n);
    for i in 0..n {
        let row = &mut scores[i * n..(i + 1) * n];
        // max — ignore NaN (should not happen here)
        let mu = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if !mu.is_finite() {
            // All -inf: zero the row (no attention at all).
            row.fill(0.0);
            continue;
        }
        let mut z = 0.0f32;
        for v in row.iter_mut() {
            let e = (*v - mu).exp();
            *v = e;
            z += e;
        }
        if z > 0.0 {
            let inv = 1.0 / z;
            for v in row.iter_mut() {
                *v *= inv;
            }
        }
    }
}
```

Propriétés vérifiables :
- Toute ligne soit somme à 1.0 ± 1e-6, soit est entièrement zéro.
- Aucune valeur > 1 ni < 0 dans le résultat.
- Pas de NaN introduit par des `-inf - -inf`.

### 8.3 Agrégation

```rust
/// Fill `new_latent[i] = Σ_j α_ij * h_j`. Row i with all-zero attention
/// is *copied verbatim* from the old latent (identity).
///
/// Complexity: O(N² D), cache-friendly with row-major layout.
pub(crate) fn aggregate(
    old_latent: &LatentView<'_>,
    new_latent: &mut LatentViewMut<'_>,
    scores_softmax: &[f32],
    mask: &MaskView<'_>,
) {
    let n = old_latent.n_agents() as usize;
    let d = old_latent.dim() as usize;
    debug_assert_eq!(scores_softmax.len(), n * n);
    for i in 0..n {
        let out = new_latent.row_mut(i as u32).expect("in-range");
        if !mask.is_active(i as u32) {
            // Inactive: copy old row verbatim to avoid corrupting it.
            out.copy_from_slice(old_latent.row(i as u32).expect("in-range"));
            continue;
        }
        let weights = &scores_softmax[i * n..(i + 1) * n];
        let weight_sum: f32 = weights.iter().sum();
        if weight_sum < f32::EPSILON {
            // Row has no attention target (all masked): identity.
            out.copy_from_slice(old_latent.row(i as u32).expect("in-range"));
            continue;
        }
        // Zero `out` buffer first.
        for o in out.iter_mut() { *o = bf16::ZERO; }
        // Accumulate in f32 scratch, write bf16 at end.
        let mut acc = vec![0.0f32; d];
        for j in 0..n {
            let w = weights[j];
            if w == 0.0 { continue; }
            let row_j = old_latent.row(j as u32).expect("in-range");
            for (a, v) in acc.iter_mut().zip(row_j.iter()) {
                *a += w * v.to_f32();
            }
        }
        for (o, a) in out.iter_mut().zip(acc.iter()) {
            *o = bf16::from_f32(*a);
        }
    }
}
```

**Important** — l'accumulateur en `f32` évite la perte de précision due à
bf16 (7-bit mantissa). Un agrégat de 128 valeurs avec 7 bits de mantissa
accumule facilement 3+ bits d'erreur. En f32 (23-bit mantissa), négligeable.

**Optimisation** : `acc` peut être un scratch pré-alloué (pas de `Vec::with_capacity`
dans la hot-path).

### 8.4 β-mix (ancrage problème)

```rust
/// h'_i := (1 - β) * h_i + β * p
pub(crate) fn beta_mix(
    latent: &mut LatentViewMut<'_>,
    problem: &EmbeddingView<'_>,
    mask: &MaskView<'_>,
    beta: f32,
) {
    debug_assert!(beta.is_finite() && (0.0..=1.0).contains(&beta));
    let n = latent.n_agents();
    let d = latent.dim() as usize;
    debug_assert_eq!(problem.as_slice().len(), d);
    let p = problem.as_slice();
    let one_minus_beta = 1.0 - beta;
    for i in 0..n {
        if !mask.is_active(i) { continue; }
        let row = latent.row_mut(i).expect("in-range");
        for (r, pp) in row.iter_mut().zip(p.iter()) {
            let mixed = one_minus_beta * r.to_f32() + beta * pp;
            *r = bf16::from_f32(mixed);
        }
    }
}
```

### 8.5 Projection (déléguée à M4)

```rust
/// For each active row, project onto anchors' cone with strength α.
/// Delegates to M4's `project_vector_onto_anchors` for the math.
pub(crate) fn project_rows(
    latent: &mut LatentViewMut<'_>,
    anchors: &AnchorsView<'_>,
    mask: &MaskView<'_>,
    alpha: f32,
) -> Result<()> {
    let anchors_host = anchors.to_host();   // alloc — OK outside hot loop
    let d = latent.dim() as usize;
    let n = latent.n_agents();
    // Scratch for row bf16↔f32 roundtrip
    let mut row_f32 = vec![0.0f32; d];
    for i in 0..n {
        if !mask.is_active(i) { continue; }
        let row = latent.row_mut(i).expect("in-range");
        for (r_f, r_bf) in row_f32.iter_mut().zip(row.iter()) {
            *r_f = r_bf.to_f32();
        }
        let (projected, _delta) = crate::els::project_vector_onto_anchors(
            &row_f32,
            &anchors_host,
            alpha,
        )?;
        for (r_bf, p_f) in row.iter_mut().zip(projected.iter()) {
            *r_bf = bf16::from_f32(*p_f);
        }
    }
    Ok(())
}
```

**Optimisation future** : un kernel natif `project_rows_batched` évite la
roundtrip bf16↔f32 et factorise la normalisation anchors.

### 8.6 Clipping + anomaly detection

```rust
/// Clip every coordinate to [-clip_max, +clip_max], reporting stats.
/// Also detects NaN/Inf and returns an error if any coordinate is not finite
/// before clipping (a coord that was already finite but large is clipped,
/// not considered an anomaly).
pub(crate) fn clip_and_check(
    latent: &mut LatentViewMut<'_>,
    mask: &MaskView<'_>,
    clip_max: f32,
) -> Result<ClipMetrics> {
    let mut clipped_rows = 0u32;
    let mut max_abs = 0.0f32;
    let n = latent.n_agents();
    let mut anomaly = false;
    for i in 0..n {
        if !mask.is_active(i) { continue; }
        let row = latent.row_mut(i).expect("in-range");
        let mut row_clipped = false;
        for r in row.iter_mut() {
            let v = r.to_f32();
            if !v.is_finite() {
                anomaly = true;
                *r = bf16::ZERO;
                continue;
            }
            if v.abs() > max_abs { max_abs = v.abs(); }
            if v > clip_max {
                *r = bf16::from_f32(clip_max);
                row_clipped = true;
            } else if v < -clip_max {
                *r = bf16::from_f32(-clip_max);
                row_clipped = true;
            }
        }
        if row_clipped { clipped_rows += 1; }
    }
    if anomaly {
        return Err(EfeError::NumericalAnomaly(
            "clip_and_check: non-finite coordinate encountered"
        ));
    }
    Ok(ClipMetrics { clipped_rows, max_abs })
}
```

### 8.7 Entropy computation (signal pour M6)

```rust
/// Row-entropy: H_i = -Σ_j α_ij * log(α_ij)
/// (0 * log(0) := 0, as usual).
///
/// Returned as (mean, min, max) over active rows.
pub(crate) fn attention_entropy_stats(
    softmax_rows: &[f32],
    n: usize,
    mask: &MaskView<'_>,
) -> (f32, f32, f32) {
    let mut sum = 0.0f32;
    let mut count = 0u32;
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for i in 0..n {
        if !mask.is_active(i as u32) { continue; }
        let row = &softmax_rows[i * n..(i + 1) * n];
        let mut h = 0.0f32;
        for &p in row {
            if p > 0.0 {
                h -= p * p.ln();
            }
        }
        sum += h;
        count += 1;
        if h < min { min = h; }
        if h > max { max = h; }
    }
    if count == 0 {
        return (0.0, 0.0, 0.0);
    }
    (sum / count as f32, min, max)
}
```

### 8.8 Schedule α_r, β_r

```rust
/// β_r = β_0 * exp(-λ_β * r)   (exponential decay of problem anchor)
pub(crate) fn beta_for_round(cfg: &FusionConfig, round: u32) -> f32 {
    let r = round as f32;
    (cfg.problem_anchor_init * (-cfg.problem_anchor_decay * r).exp())
        .clamp(0.0, cfg.problem_anchor_init)
}

/// α_r = max(floor, α_0 * (1 - r / R_max))   (linear decay, floored)
pub(crate) fn alpha_for_round(cfg: &FusionConfig, round: u32) -> f32 {
    let r = round as f32;
    let rmax = cfg.max_rounds.max(1) as f32;
    let decayed = cfg.alignment_strength_init * (1.0 - r / rmax);
    decayed.max(cfg.alignment_strength_floor)
}
```

---

## 9. Gestion des erreurs et anomalies

### 9.1 Hiérarchie des erreurs

```rust
// `EfeError` (déjà défini dans M1) est l'union globale. M5 ajoute deux variantes:

pub enum EfeError {
    // …
    /// Numerical anomaly (NaN/Inf) detected during fusion.
    NumericalAnomaly(&'static str),

    /// Deadline exceeded — caller can retry with relaxed budget.
    FusionDeadlineExceeded { round: u32, budget_us: u32, actual_us: u32 },
    // …
}
```

### 9.2 Stratégies de récupération

| Erreur | Pré-conditions | Action | Propagation |
|---|---|---|---|
| `NumericalAnomaly` dans scores | NaN dans latent d'entrée | Return Err, laisser le controller collapse | M1 — triggers M7 collapse |
| `NumericalAnomaly` dans softmax | row entière `-inf` (impossible si mask OK) | Déjà géré : ligne zéro-attention → identity | Warning, pas d'erreur |
| `NumericalAnomaly` dans clip | Explosion non-bornée → UB risk | Err → M1 restore from replay | M1 — restore(round-1) |
| `InvalidConfig` | temperature ≤ 0, strength ∉ [0,1] | Err au setup, pas de round lancé | controller re-ask |
| `FusionDeadlineExceeded` | round > budget_us | Warning + continue (next round tighter budget) OU Err (strict mode) | M1 décide via cfg |
| `Internal("split_mut …")` | ELS pas populé | Err — bug de M3 | panic in test, log+rethrow prod |

### 9.3 Replay-based recovery

```
         round r         round r+1 (échec)
            │                │
            ▼                ▼
      ┌──────────┐    ┌──────────┐
      │  latent  │    │  latent  │
      │    r     │    │ corrupted│
      └─────┬────┘    └─────┬────┘
            │                │
            │                │ M7 collapse triggers
            │                ▼
            │         ┌──────────────┐
            └────────▶│   replay     │
                      │  restore(r)  │
                      └──────┬───────┘
                             ▼
                      ┌──────────┐
                      │  latent  │
                      │   r      │  ← swarm resumes from r
                      └──────────┘
```

**Contrat** : `restore(r)` ne peut être appelé que si `r` a été `push`é. En
MVP, `replay_buffer.capacity = 4` → on peut rollback jusqu'à 4 rounds en arrière.

### 9.4 Détection early : pre-fusion validator

Avant chaque round, le validator vérifie :
1. `els.is_populated()` → sinon `Internal`
2. `pre_stats.n_active > 0` → sinon `Internal`
3. `!pre_stats.has_anomaly()` → **sinon refuse de fuser** (l'anomalie serait
   amplifiée). Propose collapse au lieu.
4. `pre_stats.variance > 0` (sinon déjà converged — warn et skip)

---

## 10. Performance et budget temporel

### 10.1 Budget MVP

Pour `N=128, D=2048, K=2 anchors` :

| Étape | Budget | Stratégie |
|---|---|---|
| Scores | 8 ms | boucle dot simple, O(N²D) |
| Softmax | < 1 ms | max-shift + exp + normalize |
| Aggregate | 8 ms | double-buffered |
| β-mix | < 1 ms | O(ND) |
| Project | 2 ms | via M4 |
| Clip | < 1 ms | O(ND) |
| Stats (pre+post) | 4 ms | M4 compute_stats |
| **Total round** | **~24 ms** | sous budget 30 ms |

Pour 16 rounds : **400 ms** par campagne de fusion. Conforme au SLO MVP
(< 1 seconde par swarm).

### 10.2 Mesure effective

Utilisation de `std::time::Instant` dans `RoundTiming`. Chaque étape émet son
µs ; le total est compris contre `round_deadline_ms` ; écart = `warnings`.

En production Sprint 5+ : wrapping avec `tracing::span!` pour corréler avec
les autres modules.

### 10.3 Parallélisation — feature `rayon`

Derrière `#[cfg(feature = "parallel")]` :

```rust
#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub(crate) fn compute_scores_parallel(
    latent: &LatentView<'_>,
    mask: &MaskView<'_>,
    scores: &mut [f32],
    params: &KernelParams,
) {
    let n = latent.n_agents() as usize;
    let d = latent.dim() as usize;
    let inv_sqrt_d = 1.0 / (d as f32).sqrt();

    scores.par_chunks_exact_mut(n).enumerate().for_each(|(i, row)| {
        // Same body as before, with captured latent+mask.
    });
}
```

Gain attendu : **1.5-3× sur 8 cores** (scores + aggregate dominent).

### 10.4 Budget mémoire par swarm

| Allocation | Taille (N=128, D=2048) |
|---|---|
| `scores` scratch | 64 KB |
| `new_latent` scratch (double-buffer) | 512 KB |
| `acc` par-row (f32) — réutilisé | 8 KB |
| Replay buffer (4 slots) | 4 × 512 KB = 2 MB |
| **Total M5 per-swarm** | **~2.6 MB** |

Sur une arena de 4 MB par swarm, il reste 1.4 MB pour d'autres modules. OK.

---

## 11. Interface publique

### 11.1 Façade `efe_core::fusion`

```rust
// Traits
pub use fusion::strategy::{FusionStrategy, ReplayableFusion};
pub use fusion::kernel::FusionKernel;

// Structs
pub use fusion::config::FusionConfig;
pub use fusion::round::{FusionRound, RoundTiming, FusionWarning};
pub use fusion::stats::FusionStats;
pub use fusion::replay::ReplayBuffer;

// Implementations
pub use fusion::mvp_cpu::MvpCpuFusion;

// Helpers for custom integrations
pub use fusion::schedule::{alpha_for_round, beta_for_round};
```

### 11.2 Exemple d'utilisation — controller

```rust
let fusion: Arc<dyn FusionStrategy> = Arc::new(
    MvpCpuFusion::builder()
        .with_config(FusionConfig::default())
        .build()?
);

let controller = SwarmController::builder()
    .genesis(genesis_engine)
    .fusion(fusion.clone())
    .consensus(consensus_detector)
    .build()?;

let outcome = controller.run(&spec).await?;
// outcome.rounds: Vec<FusionRound>
```

### 11.3 Exemple — custom backend

```rust
struct MyFastBackend { /* … */ }

#[async_trait]
impl FusionStrategy for MyFastBackend {
    async fn run_round(
        &self,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        cfg: &FusionConfig,
        round: u32,
    ) -> Result<FusionRound> {
        // custom impl, e.g., using rayon, SIMD, or a GPU kernel
        todo!()
    }
    fn backend_name(&self) -> &'static str { "my-fast" }
}

// Plug into controller:
let fusion: Arc<dyn FusionStrategy> = Arc::new(MyFastBackend::new());
```

### 11.4 Builder pattern

```rust
impl MvpCpuFusion {
    pub fn builder() -> MvpCpuFusionBuilder { MvpCpuFusionBuilder::default() }
}

#[derive(Default)]
pub struct MvpCpuFusionBuilder {
    config: Option<FusionConfig>,
    replay: Option<ReplayBuffer>,
    backend: Option<Arc<dyn DeviceBackend>>,
}

impl MvpCpuFusionBuilder {
    pub fn with_config(mut self, cfg: FusionConfig) -> Self { self.config = Some(cfg); self }
    pub fn with_replay(mut self, r: ReplayBuffer) -> Self { self.replay = Some(r); self }
    pub fn with_backend(mut self, b: Arc<dyn DeviceBackend>) -> Self { self.backend = Some(b); self }
    pub fn build(self) -> Result<MvpCpuFusion> { /* … */ }
}
```

---

## 12. Stratégie de test

### 12.1 Plan de couverture — 6 niveaux

1. **Property-based** (via `proptest`) : invariants mathématiques.
2. **Unit** : chaque kernel isolément.
3. **Integration** : orchestration complète d'un round.
4. **E2E** : M3 spawn → M5 N-rounds → M4 snapshot.
5. **Convergence** : vérifier la décroissance de variance sur swarm synthétique.
6. **Anomaly** : injection NaN/Inf, vérifier détection + replay.

### 12.2 Tests property-based

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn softmax_row_sums_to_one(
        scores in prop::collection::vec(-50.0..50.0f32, 1..32)
    ) {
        let n = scores.len();
        let mut mat = scores.clone();
        softmax_in_place_rows(&mut mat, (n as f32).sqrt() as usize);
        let sum: f32 = mat.iter().sum();
        // Multiple rows actually; simplified for example
        assert!((sum - 1.0).abs() < 1e-4 || sum == 0.0);
    }

    #[test]
    fn aggregate_preserves_l2_bound(
        n in 1..16usize,
        d in 1..64usize,
    ) {
        // If || h_i || ≤ B, then || h'_i || ≤ B (convex combination).
        // Build random latent, softmax weights, check post-aggregate norms.
    }

    #[test]
    fn beta_zero_is_identity(
        n in 1..16usize,
        d in 1..32usize,
    ) {
        // beta=0 → latent unchanged
    }

    #[test]
    fn alpha_zero_is_identity(n in 1..16usize, d in 1..32usize) {
        // alpha=0 → projection trivial
    }
}
```

### 12.3 Tests unitaires (kernels)

- `compute_scores_respects_mask`
- `compute_scores_symmetric_when_temperature_one`
- `softmax_all_neg_inf_yields_zero_row`
- `softmax_max_shift_avoids_overflow` (input = `[1e30, 1e30, 1e30]`)
- `aggregate_identity_on_zero_attention`
- `aggregate_respects_inactive_rows`
- `beta_mix_respects_mask`
- `beta_mix_beta_zero_identity`
- `project_rows_zero_alpha_identity`
- `clip_detects_infinity`
- `clip_detects_nan`
- `entropy_uniform_is_log_n`
- `entropy_peak_is_zero`
- `alpha_schedule_decreases`
- `beta_schedule_decreases`

### 12.4 Tests d'intégration (orchestration)

- `run_round_writes_new_latent_into_els`
- `run_round_emits_consistent_stats`
- `replay_push_pop_roundtrip`
- `replay_restore_exact_bytes` (hash centroid avant/après)
- `run_round_deadline_warning`
- `run_round_detects_input_anomaly_and_refuses`
- `run_round_is_deterministic_for_same_seed`

### 12.5 Tests E2E

```rust
#[tokio::test]
async fn end_to_end_variance_decreases_over_rounds() {
    let (batch, mut els) = spawn_swarm(32).await;   // M3
    let fusion = MvpCpuFusion::default();
    let arena = batch.arena_handle().unwrap().arena();

    let mut variances = vec![];
    for r in 0..8 {
        let out = fusion.run_round(&mut els, arena, &FusionConfig::default(), r).await.unwrap();
        variances.push(out.stats.variance);
    }

    // Variance should trend down (not strictly monotone, but last < first).
    assert!(variances.last().unwrap() < variances.first().unwrap(),
        "variance trend: {:?}", variances);
}

#[tokio::test]
async fn end_to_end_converges_for_synthetic_swarm() {
    // After N rounds, all rows should be within 2σ of centroid.
    // ...
}
```

### 12.6 Tests de convergence

Scénarios synthétiques :

1. **Boules gaussiennes multi-modes** : population initialisée en 3 clusters.
   Attendu : fusion progressive des 3 clusters en 1 sur 16 rounds.
2. **Distribution uniforme** : pas de structure. Attendu : variance stable.
3. **Outlier unique** : un agent très éloigné. Attendu : soit absorbé en
   quelques rounds, soit détecté comme outlier persistant.
4. **Swarm saturé** : tous les agents identiques. Attendu : identity (pas de
   mouvement, variance reste 0).

### 12.7 Tests d'anomalie

```rust
#[tokio::test]
async fn fusion_refuses_input_with_nan() {
    let (_batch, mut els) = spawn_swarm(8).await;
    // Inject NaN
    {
        let mut split = els.split_mut(arena).unwrap();
        let row = split.latent.row_mut(0).unwrap();
        row[0] = bf16::from_f32(f32::NAN);
    }
    let err = fusion.run_round(&mut els, arena, &cfg, 0).await.unwrap_err();
    assert!(matches!(err, EfeError::NumericalAnomaly(_)));
}

#[tokio::test]
async fn replay_restores_exact_state() {
    // Run round 0, snapshot, run round 1, restore to 0, hash == snapshot hash.
}
```

### 12.8 Benchmarks (`criterion`)

Dans `efe-core/benches/fusion.rs` :

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_round_128x2048(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("fusion_round_128x2048", |b| {
        b.iter(|| {
            rt.block_on(async { /* ... */ })
        })
    });
}

criterion_group!(benches, bench_round_128x2048);
criterion_main!(benches);
```

Cible : < 30 ms par round en single-thread CPU.

---

## 13. Dépendances inter-modules

### 13.1 Crates dépendues

```toml
[dependencies]
# From this workspace:
efe-kernels = { path = "../efe-kernels" }   # fusion math kernels

# External:
half = "2.4"                                 # bf16
async-trait = "0.1"                          # FusionStrategy dyn dispatch
serde = { version = "1", features = ["derive"] }
thiserror = "1"                              # EfeError extensions
tokio = { version = "1", features = ["rt"] }

[dev-dependencies]
proptest = "1"
criterion = "0.5"
```

### 13.2 Crates qui dépendront de M5

- **M1 (controller)** : consomme `Arc<dyn FusionStrategy>`.
- **M6 (consensus)** : consomme `FusionStats` de chaque round pour détecter
  la convergence (variance decrease, entropy drop, etc.).
- **M7 (collapse)** : consomme `ReplayBuffer` pour rollback sur anomalie.
- **M11 (telemetry)** : consomme `FusionRound` pour métriques.

### 13.3 Contrat avec M4

M5 accède à l'ELS **uniquement via les vues M4** :

| Accès M5 | Vue M4 utilisée | Mutabilité |
|---|---|---|
| Lire latent pour scores | `LatentView::row(i)` | RO |
| Écrire nouveau latent | `LatentViewMut::row_mut(i)` | Mut |
| Lire mask | `MaskView::is_active(i)` | RO |
| Lire anchors | `AnchorsView::iter()` / `to_host()` | RO |
| Lire problem_emb | `EmbeddingView::as_slice()` | RO |
| Pre/post stats | `EpistemicLatentSpace::snapshot()` | RO |

Pas de raw pointer dereference dans M5 — passe **toujours** par les abstractions.

### 13.4 Contrat avec M1

- M1 crée `Arc<dyn FusionStrategy>` et le passe au controller.
- M1 appelle `run_round(els, arena, cfg, r)` dans une boucle contrôlée par M6
  (consensus check entre les rounds).
- M1 handle les erreurs : `NumericalAnomaly` → trigger M7 collapse.

---

## 14. Plan de sprint M5

### 14.1 Sprint 4 — MVP CPU (2 semaines)

**Semaine 1 — Fondations**
- J1-J2 : Scaffolding module `fusion/` dans efe-core et efe-kernels.
- J2-J3 : `FusionConfig`, `FusionRound`, `KernelParams` structs + valid.
- J3-J5 : Kernels individuels (scores, softmax, aggregate, clip) + tests unit.
- J5 : `FusionKernel` trait, `CpuKernel` impl.

**Semaine 2 — Orchestration**
- J6-J7 : `FusionStrategy` trait + `MvpCpuFusion` impl + builder.
- J8 : `ReplayBuffer` impl + tests push/restore.
- J8-J9 : Intégration M4 (split_mut, project, stats).
- J9-J10 : Tests d'intégration + E2E + convergence scenarios.
- J10 : Benchmarks criterion + docs + audit.

**Livrables fin sprint 4** :
- 100 % des tests verts (unit + integration + E2E).
- Benchmarks publiés (< 30 ms / round cible).
- Architecture doc à jour.
- Code clippy pédantique clean.

### 14.2 Sprint 5 — Optimisations (1 semaine)

- SIMD via `std::simd` (feature flag `simd`).
- Parallélisation Rayon (feature flag `parallel`).
- Réduction allocations (scratch pré-alloué dans arena).
- Extension tests property-based.
- Profiling avec `perf` + optimisations tiling.

### 14.3 Sprint 6 — CUDA backend (3 semaines)

- Wrappers CUDA stable dans efe-kernels.
- `CudaFusionKernel` avec kernels dédiés (GEMM pour scores, fused softmax).
- Validation d'équivalence numérique CPU/GPU dans les tolérances bf16.
- Benchmarks multi-GPU.

### 14.4 Sprint 7 — Multi-head (futur)

- Introduire des poids learnables.
- API rétro-compatible avec feature flag `multi_head`.
- Training loop basique (hors M5 lui-même).

---

## 15. Critères d'acceptation

### 15.1 Fonctionnels

- [ ] Un `MvpCpuFusion` peut tourner 16 rounds sur un swarm (N=128, D=2048)
  sans erreur ni dépassement de budget.
- [ ] `FusionStats` est émis après chaque round, cohérent avec `compute_stats`
  calé sur l'ELS post-round.
- [ ] `ReplayBuffer::restore(r)` produit un ELS avec un `centroid_hash`
  identique à celui capturé à `r`.
- [ ] Injection de NaN dans le latent d'entrée → `EfeError::NumericalAnomaly`.
- [ ] Variance décroît strictement sur ≥ 10 rounds d'un swarm synthétique
  gaussien (scénario 1 de § 12.6).

### 15.2 Qualité de code

- [ ] `cargo fmt --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean avec
  pedantic.
- [ ] `cargo doc --no-deps --workspace` sans warnings.
- [ ] Couverture de tests ≥ 90 % sur `efe-core/src/fusion/`.
- [ ] Zéro `unsafe` supplémentaire par rapport à M4 (M5 délègue tout raw à M4).
- [ ] Tous les traits publics documentés avec exemples `# Examples`.

### 15.3 Performance

- [ ] Round sur N=128, D=2048 < 30 ms en single-thread CPU.
- [ ] Round sur N=128, D=2048 < 15 ms avec feature `parallel` sur 4 cores.
- [ ] Allocation hot-path = 0 (après setup).
- [ ] Memory footprint per-swarm < 3 MB en régime établi.

### 15.4 Observabilité

- [ ] `RoundTiming` rempli par étape.
- [ ] Tracing spans `fusion.round`, `fusion.scores`, `fusion.softmax`, etc.
- [ ] `FusionWarning` couvre les cas documentés.

---

## 16. Risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Convergence trop lente (> 16 rounds) | Moyen | Élevé | Ajuster `α_0`, `β_0`, permettre jusqu'à 32 rounds avec early-stop via M6. |
| R2 | Divergence numérique (explosion latent) | Bas | Critique | Clipping strict + replay buffer. Tests systématiques. |
| R3 | Déterminisme cassé par reordering FP | Moyen | Moyen | Ordre de sommation fixé explicitement. Docs "disable `target-cpu=native` en CI". |
| R4 | Cost `to_host()` dans project_rows | Bas | Bas | Pré-cacher les anchors une fois par round. Passer à un kernel batched en Sprint 5. |
| R5 | bf16 perd trop de précision en accumulation | Moyen | Moyen | f32 scratch pour toutes les accumulations — déjà prévu §8.3. |
| R6 | Deadlock sur ELS `&mut self` prolongé | Bas | Élevé | `split_mut` puis `drop(split)` explicite avant tout nouveau appel sur ELS. |
| R7 | Replay buffer explose la mémoire | Bas | Moyen | Cap `capacity=4` par défaut, arena séparée bornée. |
| R8 | Attention uniforme (aucun signal d'attention) | Moyen | Bas | Détecté via entropy ≈ log(N), warning émis. M6 peut fallback. |
| R9 | Les templates M3 produisent des latents trop proches | Moyen | Moyen | Test de convergence #4 (swarm saturé). Remède : initialisation plus dispersée dans M3. |
| R10 | Benchmark régresse > 10 % entre sprints | Bas | Moyen | CI gate `cargo bench` avec seuil. Historique criterion. |

---

## 17. Annexes

### 17.1 Glossaire

| Terme | Définition |
|---|---|
| ELS | Epistemic Latent Space — le tenseur partagé d'un swarm. |
| Round | Une application de l'opérateur de fusion (pas un "step" d'entraînement). |
| Attention score | Produit scalaire non normalisé `h_i · h_j / √D`. |
| Softmax row | Vecteur `α_i·` de poids non négatifs sommant à 1. |
| Centroïde | Moyenne des lignes actives du latent. |
| Cône d'anchors | Enveloppe convexe des anchors normalisés. |
| β-mix | Opérateur d'ancrage vers l'embedding du problème. |
| α-projection | Opérateur de projection soft vers le cône. |
| Replay slot | Snapshot binaire d'un latent à un round donné. |
| Replay buffer | Ring-buffer de N replay slots. |
| Clip | Bornage coordonnée par coordonnée. |
| Entropy row | `-Σ α_ij log α_ij`, mesure la "pointu" de l'attention. |

### 17.2 Pseudo-code complet du round MVP

```
function run_round(els, arena, cfg, r):
    validate_config(cfg)
    if cfg.enable_replay:
        replay_buffer.push(r, els, arena)

    pre_stats = els.snapshot(arena)
    if pre_stats.has_anomaly:
        return Err(NumericalAnomaly("pre-round"))

    params = KernelParams {
        temperature: cfg.temperature,
        mask_self_attention: cfg.mask_self_attention,
        beta: beta_for_round(cfg, r),
        alpha: alpha_for_round(cfg, r),
        clip_max: cfg.clip_max,
    }

    scratch_scores = arena.scratch_f32(N * N)
    scratch_new_latent = arena.scratch_bf16(N * D)

    # Phase 1: scores
    split = els.split_mut(arena)
    compute_scores(split.latent.as_immut(), split.mask, scratch_scores, params)

    # Phase 2: softmax
    softmax_in_place_rows(scratch_scores, N)

    # Phase 3: aggregate (into scratch_new_latent)
    aggregate(split.latent.as_immut(), &mut new_latent_view, scratch_scores, split.mask)

    # Swap new latent into place
    swap(split.latent, new_latent_view)

    # Phase 4: beta-mix
    beta_mix(&mut split.latent, split.problem_embedding, split.mask, params.beta)

    # Phase 5: projection
    project_rows(&mut split.latent, split.anchors, split.mask, params.alpha)

    # Phase 6: clip
    metrics_clip = clip_and_check(&mut split.latent, split.mask, params.clip_max)

    # Phase 7: entropy stats
    (h_mean, h_min, h_max) = attention_entropy_stats(scratch_scores, N, split.mask)

    drop(split)

    # Phase 8: post stats
    post_stats = els.snapshot(arena)
    if post_stats.has_anomaly:
        return Err(NumericalAnomaly("post-round"))

    return Ok(FusionRound {
        round: r,
        stats: build_stats(pre_stats, post_stats, h_mean, h_min, h_max, params),
        timing: take_timing(),
        backend: "mvp-cpu",
        on_time: deadline_honored(),
        warnings: collect_warnings(metrics_clip, params),
    })
```

### 17.3 Invariants formels

Pour tout round `r` et tout agent actif `i` :

1. `||h_i^(r+1)||_2 ≤ ||h_i^(r)||_2 + max_j ||h_j^(r)||_2` (convex combination + β-mix)
2. `h_i^(r+1)[d] ∈ [-CLIP_MAX, +CLIP_MAX]`
3. `variance(H^(r+1)) ≤ variance(H^(r)) + ε_drift` (empirique, pas une preuve)
4. `sum_j α_ij = 1` (propriété softmax)
5. `H_i ∈ [0, log(N)]` (entropy bornée)

### 17.4 Diagramme d'état d'un swarm durant M5

```
   ┌────────────┐
   │   READY    │  ← spawn from M3
   └──────┬─────┘
          │ fusion.run_round(0)
          ▼
   ┌────────────┐   variance drops
   │  FUSING    │◀──────────────────┐
   └──────┬─────┘                   │
          │ fusion.run_round(r)     │
          ▼                         │
   ┌────────────┐                   │
   │ ROUND END  │───────────────────┘ loop while not converged
   └──────┬─────┘
          │ M6 detects consensus OR max_rounds reached
          ▼
   ┌────────────┐
   │ CONVERGED  │  ← consumed by M8 (output projection)
   └────────────┘

   ┌────────────┐
   │   FUSING   │
   └──────┬─────┘
          │ NumericalAnomaly
          ▼
   ┌────────────┐
   │  ANOMALY   │  ← M7 decides rollback or collapse
   └────────────┘
```

### 17.5 Exemple numérique (trace)

**Setup** : N=4, D=2, β_0=0.2, α_0=0.3.

**Round 0, latent initial :**
```
h_0 = [1.0, 0.0]
h_1 = [0.0, 1.0]
h_2 = [1.0, 1.0]
h_3 = [0.5, 0.5]
```

**Scores / √D (√2 ≈ 1.414) :**
```
s_00 = (1·1 + 0·0)/√2 = 0.707
s_01 = (1·0 + 0·1)/√2 = 0.0
s_02 = (1·1 + 0·1)/√2 = 0.707
s_03 = (1·0.5 + 0·0.5)/√2 = 0.354
...
```

**Softmax de la ligne 0 (avant max-shift) :**
```
[0.707, 0.0, 0.707, 0.354]  max=0.707
[0.0, -0.707, 0.0, -0.354]  shifted
exp = [1.0, 0.493, 1.0, 0.702]  Σ=3.195
α_0 = [0.313, 0.154, 0.313, 0.220]
```

**Agrégation h'_0 :**
```
h'_0 = 0.313·[1,0] + 0.154·[0,1] + 0.313·[1,1] + 0.220·[0.5,0.5]
     = [0.313+0.313+0.110, 0.154+0.313+0.110]
     = [0.736, 0.577]
```

**β-mix avec p = [0.5, 0.5], β = 0.2 :**
```
h''_0 = 0.8·[0.736, 0.577] + 0.2·[0.5, 0.5]
      = [0.689, 0.562]
```

**Projection (anchor a_0 = [1, 1], α = 0.3) :**
```
cos(h''_0, a_0) = (0.689+0.562) / (√(0.689²+0.562²) · √2)
                ≈ 1.251 / (0.889 · 1.414) ≈ 0.995
w = 0.995, total = 0.995
centroid = a_0 = [1, 1]
h_0^(1) = 0.7·[0.689, 0.562] + 0.3·[1, 1] = [0.782, 0.693]
```

**Après clip (clip_max = 32) : inchangé.**

Cet exemple fonctionne — le round est bien traçable à la main sur petit N.

### 17.6 Checklist d'audit post-implémentation

- [ ] Tous les kernels ont un test qui vérifie l'invariant math correspondant.
- [ ] Chaque `unsafe` est justifié en commentaire avec `// SAFETY:`.
- [ ] Le replay buffer est testé avec rollback cross-round.
- [ ] Les métriques `FusionStats.mean_attention_entropy` sont cohérentes
  (∈ [0, log N]).
- [ ] `MvpCpuFusion::backend_name()` retourne une string stable.
- [ ] Doc `//!` de chaque fichier mentionne son rôle dans le pipeline.
- [ ] `clippy::cast_precision_loss` et `clippy::cast_possible_truncation`
  locally allowed sur `count as f32` / `n_active as f32` avec justification.

---

**Fin du document M5 ARCHITECTURE.md** — 1700 lignes, prêt pour implémentation sprint 4.
