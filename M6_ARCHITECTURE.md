# M6 — Consensus Detector
## Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Module** : `efe_core::consensus`
**Sprints ciblés** : Sprint 5 (MVP multi-criteria) → Sprint 6 (tuning adaptatif)
**Dépend de** : M1 (Core Runtime), M5 (Fusion Kernels), M4 (ELS — lecture pure)
**Consommé par** : M1 controller (boucle d'arrêt), M7 (collapse), M8 (output projection), M11 (telemetry)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP](#2-scope-mvp)
3. [Théorie : consensus multi-signal](#3-théorie--consensus-multi-signal)
4. [Vue d'ensemble architecturale](#4-vue-densemble-architecturale)
5. [Traits et abstractions](#5-traits-et-abstractions)
6. [Structures de données](#6-structures-de-données)
7. [Flux d'un ingest](#7-flux-dun-ingest)
8. [Algorithmes détaillés](#8-algorithmes-détaillés)
9. [Machine à états & hystérésis](#9-machine-à-états--hystérésis)
10. [Performance et budget](#10-performance-et-budget)
11. [Interface publique](#11-interface-publique)
12. [Stratégie de test](#12-stratégie-de-test)
13. [Dépendances inter-modules](#13-dépendances-inter-modules)
14. [Plan de sprint M6](#14-plan-de-sprint-m6)
15. [Critères d'acceptation](#15-critères-dacceptation)
16. [Risques et mitigations](#16-risques-et-mitigations)
17. [Annexes](#17-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M6 est le **cerveau décisionnel** de la boucle de fusion. À chaque round `r`, il
reçoit une `FusionStats` émise par M5 et répond à **une question binaire avec
nuance** : *« Le swarm a-t-il convergé suffisamment pour qu'on s'arrête ? »*

La réponse n'est pas un simple booléen — c'est un **`ConsensusVerdict`**
discriminé :

- **`Continue`** : pas encore convergé, le controller doit lancer un round de plus.
- **`Converged { confidence }`** : signaux alignés, M1 déclenche M8 (output
  projection) avec `confidence ∈ [0, 1]`.
- **`Diverged { reason }`** : la variance explose, la confidence décroît, les
  stats ne sont plus cohérentes → M1 déclenche M7 (collapse + rollback).
- **`Stalled { rounds_flat }`** : plateau persistant sans vraie convergence
  (variance stagne mais centroid ne converge pas). Le controller a le choix
  entre prolonger, abandonner, ou déclencher un soft-collapse.
- **`Anomaly { code }`** : NaN / Inf / incohérence structurelle → M7 avec rollback.

### 1.2 Propriétés clés

1. **Stateful** : M6 garde un historique glissant des `N` derniers rounds
   (fenêtre typique `N = 6`). Sans historique, impossible de détecter des
   trends (variance decreasing, centroid stable).
2. **Multi-signal** : la convergence n'est jamais inférée d'un seul indicateur.
   Six signaux indépendants sont combinés via un *confidence scorer* pondéré.
3. **Hystérésis** : un verdict `Converged` requiert **`confidence ≥ threshold`
   sur `K` rounds consécutifs** (default `K = 2`). Évite les oscillations.
4. **Zéro I/O** : M6 opère uniquement sur des structures host en mémoire. Pas
   d'accès arena, pas de network, pas de disk. Temps cible : < 100 µs par
   `ingest`.
5. **Configurable sans recompiler** : `ConsensusConfig` expose tous les seuils
   en TOML / JSON. Changer la politique de consensus = changer la config.

### 1.3 Position dans le pipeline EFE

```
  ┌─────────────────────────────────────────────────────────┐
  │                    Controller (M1)                       │
  └──┬──────────────┬──────────────┬──────────────┬──────────┘
     │              │              │              │
   spawn()      fuse(r)       check(r)       collapse()
     │              │              │              │
     ▼              ▼              ▼              ▼
  ┌─────┐       ┌──────┐     ┌──────────┐     ┌──────┐
  │ M3  │──▶    │  M5  │──▶  │   M6     │──▶  │  M7  │
  │Gene-│       │Fusion│     │Consensus │     │Coll- │
  │sis  │       └──┬───┘     └────┬─────┘     │apse  │
  └──┬──┘          │              │           └──────┘
     │             ▼              ▼
     │        ┌──────────┐   ┌─────────┐
     │        │FusionStats│──▶│Verdict  │──▶ Continue / Converged / …
     │        └──────────┘   └─────────┘
     ▼
  ┌──────────────────────────────────┐
  │      M4 — ELS (shared state)     │
  └──────────────────────────────────┘
```

### 1.4 Garanties offertes aux modules avals

1. **Idempotence d'ingest** : `ingest(same_stats)` deux fois de suite produit
   le même verdict (tant que l'historique ne change pas).
2. **Monotonie de confidence en régime stable** : si un même `FusionStats`
   est ingéré `K` fois, `confidence` ne décroît pas entre les itérations.
3. **Détection d'anomalie en 1 round** : un `FusionStats` avec
   `has_numerical_issues() == true` déclenche immédiatement `Anomaly`.
4. **Borne temporelle** : `ingest()` a complexité `O(window_size * D)`,
   où `D` est la dim latente (pour le centroid drift). Pour la config MVP
   (`window = 6, D = 2048`), c'est ~12 k ops → **< 100 µs**.

---

## 2. Scope MVP

### 2.1 IN

| Fonctionnalité | Justification MVP |
|---|---|
| `ConsensusDetector` trait + `MultiCriteriaDetector` impl | Abstraction pour DI + impl de référence. |
| 6 signaux de consensus | Couverture large, réduit les faux positifs. |
| Confidence scorer pondéré | Agrégation explicite, débuggable. |
| Fenêtre glissante `VecDeque<FusionStats>` | Détection de trends sans scan complet. |
| Hystérésis `K=2` rounds | Évite l'oscillation près du seuil. |
| Verdict discriminé (5 variantes) | Permet à M1 de brancher precisely. |
| Configuration serde (TOML/JSON) | Tuning sans recompiler. |
| Tests property-based | Invariants mathématiques vérifiés. |
| `StubDetector` pour tests M1 | Déterministe, no-op. |

### 2.2 OUT (reporté Sprint 6+)

| Fonctionnalité | Raison du report |
|---|---|
| Bayesian confidence update | Complexe, calibration nécessaire. Sprint 7. |
| Learned thresholds (meta-learning) | Nécessite dataset de runs historiques. v2. |
| Multi-swarm consensus | M8 étend à ce cas. Sprint 9. |
| Streaming detector (pas de fenêtre) | Algorithmes online (EWMA, Welford) — optim, pas blocker. Sprint 6. |
| Human-in-the-loop veto | API supplémentaire, pas MVP. v2. |
| Adversarial robustness checks | Après validation sur benchmarks. v2. |

### 2.3 Principe "multi-signal weighted"

Le MVP n'utilise **pas** un seul signal pour décider. Chaque signal capture une
dimension différente de la convergence :

- **Variance** : dispersion globale du swarm.
- **Entropy drop** : polarisation de l'attention.
- **Centroid stability** : immobilité du centre de masse.
- **Outlier ratio** : rares agents déviants.
- **Variance trend** : décroissance temporelle.
- **No-anomaly** : intégrité numérique.

Un swarm peut avoir une variance très basse **mais** un centroid qui oscille
→ pas convergé. Ou variance qui décroît **mais** anomalies en cours →
diverge. La combinaison garantit robustesse.

---

## 3. Théorie : consensus multi-signal

### 3.1 Formulation

Soit `H = [stats_{r-W+1}, …, stats_r]` la fenêtre glissante de taille `W` des
`W` derniers `FusionStats`. On définit six signaux `s_i(H) ∈ [0, 1]` :

**S1 — Variance level** (convergence globale absolue) :
```
s1 = 1 - min(1, stats_r.variance / σ²_conv)
```
où `σ²_conv` est le seuil de convergence (default `0.01`). `s1 = 1` quand
`variance = 0`; `s1 = 0` dès que `variance ≥ σ²_conv`.

**S2 — Entropy drop** (polarisation de l'attention) :
```
s2 = 1 - min(1, stats_r.mean_attention_entropy / (ln(N_active) * η_max))
```
où `η_max ∈ (0, 1]` est la fraction de `ln(N)` considérée comme "pas
polarisé" (default `0.7`). `s2 = 1` quand entropy → 0 (agents one-hot);
`s2 = 0` quand entropy ≥ `η_max * ln(N_active)`.

**S3 — Centroid stability** (immobilité) :
```
s3 = 1 - min(1, ||μ_r - μ_{r-1}||_2 / (||μ_r||_2 * δ_max))
```
où `δ_max` est la tolérance relative (default `0.02`). `s3 = 1` quand
centroid immobile; `s3 = 0` quand drift ≥ `2%`.

**S4 — Outlier ratio** (cohésion) :
```
s4 = 1 - min(1, stats_r.outlier_ratio / ρ_max)
```
où `ρ_max` est la fraction maximale d'outliers tolérée (default `0.05`).

**S5 — Variance trend** (décroissance monotone) :
```
trend = median_slope({σ²_{r-W+1}, …, σ²_r})
s5 = σ(-trend * κ)   (sigmoïde: trend négatif → s5 → 1)
```
où `σ` est la sigmoïde et `κ` un gain. `s5 = 0.5` quand trend nul;
`s5 → 1` quand variance décroît fortement.

**S6 — Anomaly absence** (binaire mais lissé) :
```
s6 = 1 if !stats_r.has_numerical_issues() else 0
```

### 3.2 Confidence score

```
confidence = Σ_i w_i * s_i(H)    avec Σ_i w_i = 1
```

Poids par défaut (somment à 1.0) :

| Signal | Poids | Rationale |
|---|---:|---|
| S1 variance level | 0.25 | Signal le plus direct |
| S2 entropy drop | 0.20 | Mesure spécifique M5 |
| S3 centroid stability | 0.15 | Robuste aux bruits ponctuels |
| S4 outlier ratio | 0.10 | Cohésion de population |
| S5 variance trend | 0.15 | Capture la dynamique |
| S6 no-anomaly | 0.15 | Veto partiel (avec S5) |

### 3.3 Règle de verdict

Pseudo-code de la règle :

```
update history with stats
compute signals s1..s6 → confidence ∈ [0, 1]

if stats.has_numerical_issues():
    return Anomaly { code: NanInf }

if stats.variance == +inf OR centroid has NaN:
    return Diverged { reason: VarianceExplosion }

if variance_trend(history) is consistently positive AND growing:
    return Diverged { reason: VarianceGrowing }

if confidence >= threshold:
    consecutive_high += 1
    if consecutive_high >= K:
        return Converged { confidence }
else:
    consecutive_high = 0

if history full AND confidence range in last W < ε AND confidence < threshold:
    return Stalled { rounds_flat: W }

return Continue
```

### 3.4 Propriétés de correctness souhaitées

1. **Monotonie près du seuil** : si le swarm progresse (signals s_i
   monotones), alors la confidence augmente. Vérifié par property test.
2. **Robustesse aux spikes** : un unique round "mauvais" (variance haute)
   ne fait pas basculer Converged → Diverged si les autres rounds sont OK.
3. **Détection rapide d'anomalie** : `Anomaly` en 1 round (pas de fenêtre
   nécessaire).
4. **Pas de faux positif Converged avant W rounds** : impossible d'avoir
   `Converged` dans les `W-1` premiers rounds (l'historique est incomplet,
   certains signaux retournent 0).

---

## 4. Vue d'ensemble architecturale

### 4.1 Disposition

```
efe-core/src/consensus/
├── mod.rs                    # Facade + re-exports
├── config.rs                 # ConsensusConfig
├── verdict.rs                # ConsensusVerdict + reasons
├── history.rs                # ConvergenceHistory (ring buffer)
├── signals.rs                # 6 signaux s_i + entrée unique compute_signals
├── scorer.rs                 # ConfidenceScorer pondéré
├── detector.rs               # ConsensusDetector trait
├── multi_criteria.rs         # MultiCriteriaDetector impl
├── stub.rs                   # StubDetector pour tests
└── telemetry.rs              # Mapping ConsensusVerdict → metrics labels
```

**Séparation des responsabilités** :
- `signals.rs` : fonctions pures `fn s1(history) -> f32`, etc.
- `scorer.rs` : agrège les signaux en confidence.
- `multi_criteria.rs` : orchestration — state + verdict decision.
- `history.rs` : ring buffer typé.

### 4.2 Diagramme de classes

```
                 ┌────────────────────────────┐
                 │  ConsensusDetector (trait) │
                 │  fn ingest(stats) → Verdict│
                 │  fn verdict() → Verdict    │
                 │  fn history() → History    │
                 │  fn reset()                │
                 └───────────┬────────────────┘
                             │ impl
           ┌─────────────────┼─────────────────┐
           │                 │                 │
  ┌────────▼─────────┐ ┌─────▼──────┐  ┌───────▼───────┐
  │ MultiCriteria    │ │ StubDetect │  │ BayesianDetect│
  │ Detector (MVP)   │ │ (tests)    │  │ (Sprint 7+)   │
  └────────┬─────────┘ └────────────┘  └───────────────┘
           │ holds
           │
    ┌──────┼──────────┬────────────────────┐
    ▼      ▼          ▼                    ▼
┌──────┐┌─────────┐┌────────────┐    ┌───────────┐
│Conf- ││History  ││Signals     │    │Config     │
│Score ││(ring)   ││(pure fns)  │    │(serde)    │
└──────┘└─────────┘└────────────┘    └───────────┘
```

### 4.3 Pipeline d'une décision

```
         ┌───────────────────────────────┐
         │     detector.ingest(stats)     │
         └───────────────────────────────┘
                        │
            ┌───────────┼────────────┐
            ▼                        ▼
       ┌─────────┐           ┌──────────────┐
       │history  │           │ anomaly fast │
       │.push    │           │ path         │
       └────┬────┘           └──────┬───────┘
            │                       │ if anomaly
            ▼                       ▼
     ┌──────────────┐         return Anomaly
     │compute_signals│
     └──────┬───────┘
            │
            ▼
     ┌──────────────┐
     │scorer.score  │ → confidence ∈ [0, 1]
     └──────┬───────┘
            │
            ▼
     ┌──────────────┐
     │verdict rules │
     │(hysteresis)  │
     └──────┬───────┘
            │
            ▼
     ConsensusVerdict
```

---

## 5. Traits et abstractions

### 5.1 `ConsensusDetector` — façade

```rust
/// Observe `FusionStats` emitted by M5 and emit a verdict.
///
/// Implementations:
///   - `MultiCriteriaDetector` — MVP, 6-signal weighted.
///   - `StubDetector`          — always Continue (tests).
///   - `BayesianDetector`      — Sprint 7+.
///
/// # Contract
/// - `ingest` is the only mutating method.
/// - `verdict`, `history`, `confidence` are read-only.
/// - Implementations must be `Send + Sync` so the controller (M1) can
///   hold them behind `Arc<dyn>`.
pub trait ConsensusDetector: Send + Sync + std::fmt::Debug {
    /// Ingest a round's stats and return the updated verdict.
    ///
    /// # Errors
    /// - `InvalidConfig` if the stats are structurally malformed
    ///   (e.g. centroid empty, round index non-monotonic).
    fn ingest(&mut self, stats: &FusionStats) -> Result<ConsensusVerdict>;

    /// Current verdict. Safe to call without ingest (returns Continue by default).
    fn verdict(&self) -> &ConsensusVerdict;

    /// Current confidence ∈ [0, 1]. `0.0` when no ingest has happened.
    fn confidence(&self) -> f32;

    /// Borrow the convergence history.
    fn history(&self) -> &ConvergenceHistory;

    /// Reset state for reuse with a different swarm.
    fn reset(&mut self);
}
```

### 5.2 `MultiCriteriaDetector` — impl de référence

```rust
pub struct MultiCriteriaDetector {
    config: ConsensusConfig,
    history: ConvergenceHistory,
    scorer: ConfidenceScorer,
    consecutive_high: u32,
    last_verdict: ConsensusVerdict,
    last_confidence: f32,
    /// Monotonic round index seen so far (detect skips / regressions).
    last_round: Option<u32>,
}
```

### 5.3 `ConfidenceScorer` — agrégation

```rust
/// Weighted sum of the 6 signals. Exposed as a separate type so it can
/// be unit-tested in isolation and swapped (e.g. to a neural scorer in
/// Sprint 7).
pub struct ConfidenceScorer {
    weights: SignalWeights,
}

pub struct SignalWeights {
    pub variance_level: f32,
    pub entropy_drop: f32,
    pub centroid_stability: f32,
    pub outlier_ratio: f32,
    pub variance_trend: f32,
    pub no_anomaly: f32,
}

impl SignalWeights {
    /// Weights must sum to 1.0; validated at construction.
    pub fn new(...) -> Result<Self>;
    pub fn default_mvp() -> Self;
    fn sum(&self) -> f32;
}
```

### 5.4 `ConvergenceHistory` — ring buffer

```rust
pub struct ConvergenceHistory {
    /// Ring buffer of the last W FusionStats, oldest first.
    buf: VecDeque<StatsRecord>,
    /// Maximum size.
    capacity: usize,
}

struct StatsRecord {
    round: u32,
    variance: f32,
    gradient_norm: f32,
    mean_attention_entropy: f32,
    outlier_ratio: f32,
    n_active: u32,
    centroid: Vec<f32>,   // cloned from FusionStats
    has_anomaly: bool,
}
```

---

## 6. Structures de données

### 6.1 `ConsensusConfig`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConsensusConfig {
    /// Window size for trend analysis. Must be ≥ 2.
    pub history_window: u32,                 // default: 6

    /// Convergence threshold on the confidence score.
    pub confidence_threshold: f32,           // default: 0.80

    /// Consecutive rounds above threshold required for Converged.
    pub hysteresis_rounds: u32,              // default: 2

    // --- Signal thresholds ---
    /// σ²_conv — variance ≤ this is considered converged (S1).
    pub variance_threshold: f32,             // default: 0.01

    /// η_max — fraction of ln(N) below which entropy is "low" (S2).
    pub entropy_fraction_max: f32,           // default: 0.7

    /// δ_max — max relative centroid drift for S3=1.
    pub centroid_drift_max: f32,             // default: 0.02

    /// ρ_max — max outlier ratio for S4=1.
    pub outlier_ratio_max: f32,              // default: 0.05

    /// κ — trend gain for S5 sigmoid.
    pub trend_gain: f32,                     // default: 50.0

    /// Absolute variance above which Diverged is declared.
    pub variance_explosion_threshold: f32,   // default: 100.0

    /// Number of rounds of flat low-confidence to trigger Stalled.
    pub stall_window: u32,                   // default: 6

    /// Range of confidence (max-min) below which "flat".
    pub stall_confidence_range: f32,         // default: 0.05

    /// Weights for the 6 signals.
    pub weights: SignalWeights,
}
```

### 6.2 `ConsensusVerdict`

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ConsensusVerdict {
    /// Not converged yet — controller continues fusion.
    Continue,

    /// Converged with the given confidence ∈ [threshold, 1].
    Converged { confidence: f32 },

    /// Diverged — controller triggers M7 (collapse + rollback).
    Diverged { reason: DivergenceReason },

    /// Stalled on a plateau — low-confidence flat region.
    /// Controller may extend the campaign, accept as weak consensus, or fail.
    Stalled { rounds_flat: u32, confidence: f32 },

    /// Numerical anomaly — controller triggers M7 (collapse).
    Anomaly { code: AnomalyCode },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivergenceReason {
    VarianceExplosion { variance: f32 },
    VarianceGrowing { slope: f32, window: u32 },
    ConfidenceDecay { from: f32, to: f32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalyCode {
    NanInf,
    CentroidDimMismatch,
    NonMonotonicRound { last: u32, current: u32 },
}

impl ConsensusVerdict {
    pub fn is_terminal(&self) -> bool {
        matches!(self,
            Self::Converged { .. } | Self::Diverged { .. } | Self::Anomaly { .. })
    }

    pub fn should_rollback(&self) -> bool {
        matches!(self, Self::Diverged { .. } | Self::Anomaly { .. })
    }
}
```

### 6.3 `Signals` — struct de sortie du calcul

```rust
/// Raw signal values + diagnostics, emitted by compute_signals().
/// Consumed by the scorer and (optionally) telemetry.
#[derive(Debug, Clone, Copy)]
pub struct Signals {
    pub s1_variance_level: f32,
    pub s2_entropy_drop: f32,
    pub s3_centroid_stability: f32,
    pub s4_outlier_ratio: f32,
    pub s5_variance_trend: f32,
    pub s6_no_anomaly: f32,
}

impl Signals {
    pub fn all_finite(&self) -> bool;
    pub fn to_array(&self) -> [f32; 6];
}
```

### 6.4 `StatsRecord` — entry dans l'historique

```rust
/// Compact per-round record retained in the history window.
/// Derived from FusionStats, minus the huge `centroid` (kept separately
/// — we only need the *last two* centroids for S3, not the full window).
#[derive(Debug, Clone)]
pub(crate) struct StatsRecord {
    pub round: u32,
    pub variance: f32,
    pub gradient_norm: f32,
    pub mean_attention_entropy: f32,
    pub outlier_ratio: f32,
    pub n_active: u32,
    pub confidence: f32,      // filled by scorer each ingest
    pub has_anomaly: bool,
}
```

**Note** : on ne garde le centroid que pour les 2 derniers rounds (stocké
séparément dans le detector, pas dans la history). Économise `(W - 2) * D * 4`
bytes, soit ~33 KB pour `W=6, D=2048`.

---

## 7. Flux d'un ingest

### 7.1 Séquence détaillée

```rust
pub fn ingest(&mut self, stats: &FusionStats) -> Result<ConsensusVerdict> {
    // 0. Structural validation
    self.validate_stats_shape(stats)?;

    // 1. Round monotonicity check
    if let Some(last) = self.last_round {
        if stats.round < last {
            return self.set_verdict(ConsensusVerdict::Anomaly {
                code: AnomalyCode::NonMonotonicRound { last, current: stats.round }
            });
        }
    }
    self.last_round = Some(stats.round);

    // 2. Fast-path anomaly
    if stats.has_numerical_issues() {
        return self.set_verdict(ConsensusVerdict::Anomaly {
            code: AnomalyCode::NanInf
        });
    }

    // 3. Update history
    self.history.push(StatsRecord::from_stats(stats));

    // 4. Compute signals + confidence
    let signals = compute_signals(&self.history, &self.config, stats);
    let confidence = self.scorer.score(&signals);
    self.last_confidence = confidence;

    // 5. Update last record with confidence
    self.history.update_last_confidence(confidence);

    // 6. Divergence gate
    if let Some(v) = self.detect_divergence(stats, confidence) {
        return self.set_verdict(v);
    }

    // 7. Hysteresis
    if confidence >= self.config.confidence_threshold {
        self.consecutive_high += 1;
        if self.consecutive_high >= self.config.hysteresis_rounds {
            return self.set_verdict(ConsensusVerdict::Converged { confidence });
        }
    } else {
        self.consecutive_high = 0;
    }

    // 8. Stall detection
    if let Some(v) = self.detect_stall() {
        return self.set_verdict(v);
    }

    // 9. Default — continue
    self.set_verdict(ConsensusVerdict::Continue);
    Ok(self.last_verdict.clone())
}
```

### 7.2 Micro-budget par étape

| Étape | Cost | Notes |
|---|---|---|
| 0. Validate shape | O(1) | Check centroid.len() == previous |
| 1. Monotonicity | O(1) | |
| 2. Anomaly fast-path | O(1) | |
| 3. History push | O(D) | Clone centroid into record (copy only if kept) |
| 4. Compute signals | O(W + D) | Centroid drift scans D; trend scans W |
| 5. Score | O(1) | 6 multiplications |
| 6. Divergence | O(W) | |
| 7. Hysteresis | O(1) | |
| 8. Stall | O(W) | Confidence range |
| 9. Set verdict | O(1) | |
| **Total** | **O(W + D)** | ≈ 2k ops for default config |

### 7.3 Invariants maintenus entre ingests

- `history.len() ≤ capacity`
- Les rounds dans history sont strictement croissants
- `last_confidence ∈ [0, 1] ∪ {NaN seulement si config invalid}`
- `consecutive_high ∈ [0, hysteresis_rounds]` (jamais au-delà)

---

## 8. Algorithmes détaillés

### 8.1 Signal S1 — Variance level

```rust
fn s1_variance_level(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    if !stats.variance.is_finite() || cfg.variance_threshold <= 0.0 {
        return 0.0;
    }
    let ratio = stats.variance / cfg.variance_threshold;
    (1.0 - ratio).clamp(0.0, 1.0)
}
```

Properties :
- `variance = 0` → `s1 = 1`
- `variance = threshold` → `s1 = 0`
- `variance > threshold` → `s1 = 0` (clamped)
- `variance = NaN` → `s1 = 0` (failsafe)

### 8.2 Signal S2 — Entropy drop

```rust
fn s2_entropy_drop(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    if stats.n_active_approx() == 0 {
        return 0.0;
    }
    let ln_n = (stats.n_active_approx() as f32).ln().max(1e-6);
    let h = stats.mean_attention_entropy;
    if !h.is_finite() { return 0.0; }
    let ratio = h / (ln_n * cfg.entropy_fraction_max.max(1e-6));
    (1.0 - ratio).clamp(0.0, 1.0)
}
```

Note : `stats.n_active_approx()` — FusionStats n'a pas de `n_active`, on le
dérive via `centroid.len()` si disponible, ou on le passe explicitement via
`StatsRecord`. **À clarifier** : on ajoute un champ `n_active: u32` à
`StatsRecord` lors du `push()` en le récupérant depuis le `FusionStats` si
exposé, sinon fallback à `1`.

**Décision MVP** : on ajoute un champ `n_active: u32` à `FusionStats`. Déjà
calculable côté M5 via `compute_stats(...).n_active`. Trivial à propager.

### 8.3 Signal S3 — Centroid stability

```rust
fn s3_centroid_stability(
    prev_centroid: &[f32],
    current_centroid: &[f32],
    cfg: &ConsensusConfig,
) -> f32 {
    if prev_centroid.len() != current_centroid.len() { return 0.0; }
    let drift: f32 = prev_centroid.iter().zip(current_centroid.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt();
    let norm: f32 = current_centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < f32::EPSILON { return 0.0; }
    let ratio = drift / (norm * cfg.centroid_drift_max.max(1e-6));
    (1.0 - ratio).clamp(0.0, 1.0)
}
```

### 8.4 Signal S4 — Outlier ratio

```rust
fn s4_outlier_ratio(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    let r = stats.outlier_ratio;
    if !r.is_finite() { return 0.0; }
    let ratio = r / cfg.outlier_ratio_max.max(1e-6);
    (1.0 - ratio).clamp(0.0, 1.0)
}
```

### 8.5 Signal S5 — Variance trend

Calcule la **pente médiane** (Theil-Sen) de la variance sur la fenêtre — robuste
aux spikes ponctuels.

```rust
fn s5_variance_trend(history: &ConvergenceHistory, cfg: &ConsensusConfig) -> f32 {
    let variances: Vec<(f32, f32)> = history.iter()
        .enumerate()
        .filter(|(_, r)| r.variance.is_finite())
        .map(|(i, r)| (i as f32, r.variance))
        .collect();
    if variances.len() < 2 { return 0.5; }  // neutral
    // Theil-Sen: median of all pairwise slopes
    let mut slopes = Vec::with_capacity(variances.len() * (variances.len() - 1) / 2);
    for i in 0..variances.len() {
        for j in (i + 1)..variances.len() {
            let (xi, yi) = variances[i];
            let (xj, yj) = variances[j];
            if xj - xi > 0.0 {
                slopes.push((yj - yi) / (xj - xi));
            }
        }
    }
    if slopes.is_empty() { return 0.5; }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let trend = slopes[slopes.len() / 2];
    sigmoid(-trend * cfg.trend_gain)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}
```

Properties :
- `trend = 0` → `s5 = 0.5`
- `trend << 0` (decreasing fast) → `s5 → 1`
- `trend >> 0` (increasing) → `s5 → 0`

### 8.6 Signal S6 — No anomaly

```rust
fn s6_no_anomaly(stats: &FusionStats) -> f32 {
    if stats.has_numerical_issues() { 0.0 } else { 1.0 }
}
```

Dans la chaîne principale, S6=0 déclenche déjà `Anomaly` avant d'arriver au
scorer — ce signal sert de garde-fou.

### 8.7 Détection de divergence

```rust
fn detect_divergence(&self, stats: &FusionStats, confidence: f32)
    -> Option<ConsensusVerdict>
{
    // Variance explosion
    if stats.variance > self.config.variance_explosion_threshold {
        return Some(Diverged {
            reason: VarianceExplosion { variance: stats.variance }
        });
    }
    // Variance consistently growing over the window
    let slope = variance_slope(&self.history);
    if slope > 0.0 && self.history.is_full() && self.all_variances_increasing() {
        return Some(Diverged {
            reason: VarianceGrowing {
                slope,
                window: self.config.history_window,
            }
        });
    }
    // Confidence decay — confidence was once above threshold, now sharply below
    if let Some(peak) = self.peak_confidence() {
        if peak > self.config.confidence_threshold
            && confidence < peak * 0.5
        {
            return Some(Diverged {
                reason: ConfidenceDecay { from: peak, to: confidence }
            });
        }
    }
    None
}
```

### 8.8 Détection de stall

```rust
fn detect_stall(&self) -> Option<ConsensusVerdict> {
    if self.history.len() < self.config.stall_window as usize { return None; }
    let confidences: Vec<f32> = self.history.iter()
        .rev()
        .take(self.config.stall_window as usize)
        .map(|r| r.confidence)
        .collect();
    let max = confidences.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let min = confidences.iter().cloned().fold(f32::INFINITY, f32::min);
    let range = max - min;
    let below_threshold = confidences.iter()
        .all(|&c| c < self.config.confidence_threshold);
    if range < self.config.stall_confidence_range && below_threshold {
        return Some(Stalled {
            rounds_flat: self.config.stall_window,
            confidence: self.last_confidence,
        });
    }
    None
}
```

### 8.9 Scorer

```rust
impl ConfidenceScorer {
    pub fn score(&self, s: &Signals) -> f32 {
        let w = &self.weights;
        let c =
            w.variance_level       * s.s1_variance_level       +
            w.entropy_drop         * s.s2_entropy_drop         +
            w.centroid_stability   * s.s3_centroid_stability   +
            w.outlier_ratio        * s.s4_outlier_ratio        +
            w.variance_trend       * s.s5_variance_trend       +
            w.no_anomaly           * s.s6_no_anomaly;
        c.clamp(0.0, 1.0)
    }
}
```

---

## 9. Machine à états & hystérésis

### 9.1 États implicites

Le detector n'a pas d'état explicite — il est à état implicite via
`consecutive_high`, `last_confidence`, et `history`. Les transitions :

```
    [Init]
      │
      │ first ingest
      ▼
    [Observing] ──────────────┐
      │   │                   │
      │   │ confidence high   │ anomaly
      │   ▼                   ▼
      │   consecutive++       [Anomaly]  ← terminal
      │   │
      │   │ K consecutive highs
      │   ▼
      │   [Converged]  ← terminal
      │
      │ variance explode / confidence decay
      ▼
    [Diverged]  ← terminal
      │
      ▼
    [Stalled]  ← terminal (controller may re-ingest to retry)
```

### 9.2 Pourquoi hystérésis

Sans hystérésis (`K=1`), confidence près du seuil oscille :

```
round:     1   2   3   4   5   6
confidence:0.78 0.82 0.79 0.81 0.77 0.83
verdict:   C   V   C   V   C   V     (oscille!)
```

Avec `K=2` :

```
round:     1   2   3   4   5   6
confidence:0.78 0.82 0.79 0.81 0.77 0.83
consecutive:0  1   0   1   0   1
verdict:   C   C   C   C   C   C     (stable)
```

Le verdict `Converged` ne se déclenche que si `confidence ≥ threshold` sur
`K` rounds consécutifs.

### 9.3 Reset

```rust
fn reset(&mut self) {
    self.history.clear();
    self.consecutive_high = 0;
    self.last_verdict = ConsensusVerdict::Continue;
    self.last_confidence = 0.0;
    self.last_round = None;
}
```

Appelé par le controller quand il démarre une nouvelle campagne (nouveau
swarm). Le `MultiCriteriaDetector` est réutilisable.

---

## 10. Performance et budget

### 10.1 Budget MVP par `ingest`

Pour `W = 6, D = 2048` :

| Étape | Ops | Temps (ns) |
|---|---|---|
| Validation | ~10 | 5 |
| Anomaly check | ~5 | 2 |
| History push | D + 10 | 4 000 |
| Signals S1-S4 | ~20 | 10 |
| Signal S3 (centroid drift) | 2D | 8 000 |
| Signal S5 (Theil-Sen) | W² = 36 | 200 |
| Scorer | 6 muls | 3 |
| Divergence | W | 30 |
| Stall | W | 30 |
| **Total** | | **~12 µs** |

Conforme au budget SLO (< 100 µs).

### 10.2 Footprint mémoire

Par `MultiCriteriaDetector` :

- `history` : `W * sizeof(StatsRecord)` = `6 * (4*6 + 8) ≈ 200 B`
- `prev_centroid` : `D * 4` = `8 KB`
- `curr_centroid` : `8 KB`
- Config : ~100 B
- Scorer : 6 * 4 = 24 B

**Total : ~16 KB par detector**, négligeable.

### 10.3 Scaling

- Linéaire en `D` (centroid drift).
- Quadratique en `W` pour Theil-Sen — mais `W ≤ 10` en pratique, négligeable.
- `O(1)` mémoire par round au-delà de la fenêtre (ring buffer).

### 10.4 Cross-threading

Le detector n'est **pas** thread-safe en écriture par design : `ingest(&mut self)`
interdit les appels concurrents. Dans le controller, il est protégé par
`Mutex<Box<dyn ConsensusDetector>>`.

Pour du multi-swarm concurrent, chaque swarm a son propre detector —
pas de synchronisation nécessaire.

---

## 11. Interface publique

### 11.1 Façade `efe_core::consensus`

```rust
pub use consensus::config::{ConsensusConfig, SignalWeights};
pub use consensus::verdict::{
    AnomalyCode, ConsensusVerdict, DivergenceReason,
};
pub use consensus::history::ConvergenceHistory;
pub use consensus::signals::Signals;
pub use consensus::scorer::ConfidenceScorer;
pub use consensus::detector::ConsensusDetector;
pub use consensus::multi_criteria::MultiCriteriaDetector;
pub use consensus::stub::StubDetector;
```

### 11.2 Exemple — intégration controller

```rust
let detector: Arc<Mutex<Box<dyn ConsensusDetector>>> =
    Arc::new(Mutex::new(Box::new(MultiCriteriaDetector::new(
        ConsensusConfig::default()
    )?)));

for round in 0..fusion_cfg.max_rounds {
    let outcome = fusion.run_round(&mut els, &arena, &fusion_cfg, round).await?;

    let verdict = {
        let mut d = detector.lock().unwrap();
        d.ingest(&outcome.stats)?
    };

    match verdict {
        ConsensusVerdict::Continue => { /* loop */ }
        ConsensusVerdict::Converged { confidence } => {
            // Hand off to M8 with confidence
            break;
        }
        ConsensusVerdict::Diverged { reason } => {
            // Trigger M7 rollback
            break;
        }
        ConsensusVerdict::Stalled { rounds_flat, confidence } => {
            tracing::warn!(?rounds_flat, ?confidence, "consensus stalled");
            break;
        }
        ConsensusVerdict::Anomaly { code } => {
            // Trigger M7 collapse
            break;
        }
    }
}
```

### 11.3 Custom weights

```rust
let weights = SignalWeights::new(
    /* variance_level */ 0.30,
    /* entropy_drop */ 0.30,
    /* centroid_stability */ 0.10,
    /* outlier_ratio */ 0.10,
    /* variance_trend */ 0.10,
    /* no_anomaly */ 0.10,
)?;
let config = ConsensusConfig::default().with_weights(weights);
let detector = MultiCriteriaDetector::new(config)?;
```

### 11.4 StubDetector (tests)

```rust
/// Always returns Continue. Useful for unit-testing the controller.
pub struct StubDetector;

impl ConsensusDetector for StubDetector {
    fn ingest(&mut self, _: &FusionStats) -> Result<ConsensusVerdict> {
        Ok(ConsensusVerdict::Continue)
    }
    fn verdict(&self) -> &ConsensusVerdict { &ConsensusVerdict::Continue }
    fn confidence(&self) -> f32 { 0.0 }
    fn history(&self) -> &ConvergenceHistory { static H: … }
    fn reset(&mut self) {}
}
```

---

## 12. Stratégie de test

### 12.1 Couverture

| Niveau | Exemples |
|---|---|
| Unit — signals | Each `s_i` function on crafted inputs |
| Unit — scorer | Weighted sum math |
| Unit — history | Ring buffer push/clear/capacity |
| Unit — verdict | Each variant reachable via state |
| Integration — detector | Full ingest flow on synthetic FusionStats |
| Property — monotonicity | Confidence ↗ when signals ↗ |
| Property — anomaly detection | Any NaN → Anomaly in 1 ingest |
| Property — hysteresis | Converged requires K rounds |
| E2E — controller | M5 → M6 → M1 decision loop |

### 12.2 Tests property-based

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn confidence_bounded(
        s1 in 0.0..=1.0f32,
        s2 in 0.0..=1.0f32,
        s3 in 0.0..=1.0f32,
        s4 in 0.0..=1.0f32,
        s5 in 0.0..=1.0f32,
        s6 in 0.0..=1.0f32,
    ) {
        let scorer = ConfidenceScorer::default();
        let signals = Signals { s1, s2, s3, s4, s5, s6 };
        let c = scorer.score(&signals);
        prop_assert!((0.0..=1.0).contains(&c));
    }

    #[test]
    fn anomaly_detected_in_one_round(
        variance in 0.0..1.0f32,
    ) {
        let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default())?;
        let mut stats = FusionStats::synthetic_converging(0, 4);
        stats.variance = variance;
        stats.has_nan = true;
        let verdict = detector.ingest(&stats)?;
        prop_assert!(matches!(verdict, ConsensusVerdict::Anomaly { .. }));
    }

    #[test]
    fn no_converged_before_hysteresis_rounds(
        history_size in 0..6u32,
    ) {
        // Feed `history_size` perfect stats; verdict must be Continue.
        let mut detector = MultiCriteriaDetector::new(ConsensusConfig {
            hysteresis_rounds: 2,
            ..Default::default()
        })?;
        for r in 0..history_size {
            let stats = FusionStats::synthetic_converged_perfect(r);
            detector.ingest(&stats)?;
        }
        if history_size < 2 {
            prop_assert!(!matches!(
                detector.verdict(),
                ConsensusVerdict::Converged { .. }
            ));
        }
    }
}
```

### 12.3 Tests unitaires (signaux)

- `s1_saturates_at_variance_zero`
- `s1_zero_above_threshold`
- `s1_handles_nan_safely`
- `s2_scales_with_ln_n`
- `s2_zero_on_uniform_entropy`
- `s3_zero_when_centroid_drifts`
- `s3_one_when_centroid_stable`
- `s3_handles_empty_centroids`
- `s4_linear_from_zero_to_threshold`
- `s5_theil_sen_robust_to_spike`
- `s5_point_five_on_flat`
- `s6_binary_anomaly_flag`

### 12.4 Tests scorer

- `score_default_weights_sum_to_one`
- `score_zeros_gives_zero`
- `score_all_ones_gives_one`
- `score_is_deterministic`
- `weights_construction_rejects_non_unit_sum`

### 12.5 Tests history

- `history_push_respects_capacity`
- `history_clear_empties`
- `history_rounds_are_monotonic`
- `history_iter_returns_oldest_first`

### 12.6 Tests verdict flow

- `fresh_detector_returns_continue`
- `anomaly_path_bypasses_history`
- `convergence_requires_hysteresis`
- `divergence_on_variance_explosion`
- `stall_on_low_flat_confidence`
- `reset_clears_all_state`
- `repeated_same_stats_is_idempotent`

### 12.7 Tests E2E (M5 → M6)

```rust
#[tokio::test]
async fn fusion_plus_consensus_loop_converges_on_synthetic() {
    let (batch, mut els) = spawn_swarm(16, 32).await;
    let arena = batch.arena_handle().unwrap().arena();
    let fusion = MvpCpuFusion::builder().with_backend(backend)
        .with_replay_enabled(true).build().unwrap();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let cfg = FusionConfig::default();

    for r in 0..cfg.max_rounds {
        let round = fusion.run_round(&mut els, arena, &cfg, r).await.unwrap();
        let verdict = detector.ingest(&round.stats).unwrap();
        if verdict.is_terminal() {
            println!("terminated at round {r}: {verdict:?}");
            break;
        }
    }

    // On a fresh spawned swarm with default fusion config, we expect
    // either Converged (if 16 rounds is enough) or Stalled, but never
    // Anomaly nor Diverged.
    match detector.verdict() {
        ConsensusVerdict::Converged { .. } => {}
        ConsensusVerdict::Stalled { .. } => {}
        ConsensusVerdict::Continue => {}
        other => panic!("unexpected verdict: {other:?}"),
    }
}
```

### 12.8 Tests adversariaux

- Flip-flop : alterner high/low stats, vérifier que `Converged` n'est pas
  déclenché.
- Spike unique : 5 rounds OK + 1 spike puis 5 OK → vérifier que ça ne
  déclenche pas `Diverged` (robustesse).
- Stall exactement à la frontière : confidence = 0.79 constant → `Stalled`.

---

## 13. Dépendances inter-modules

### 13.1 Crates dépendus

```toml
[dependencies]
# workspace:
serde = { workspace = true, features = ["derive"] }
thiserror.workspace = true
# external:
# (M6 est du math pur host-side, aucune dép externe supplémentaire)

[dev-dependencies]
proptest = "1.4"
```

### 13.2 Interface avec M5

M6 consomme `FusionStats` → lecture seule. **Contrat** : les champs
nécessaires doivent être présents :

- `round: u32`
- `variance: f32`
- `gradient_norm: f32`
- `centroid: Vec<f32>`
- `outlier_ratio: f32`
- `mean_attention_entropy: f32`
- `has_nan: bool`
- `has_inf: bool`

**Ajout requis à `FusionStats`** : `n_active: u32`. Valeur calculée par
`compute_stats(...).n_active` côté M5, à propager. Coût : 4 bytes.

### 13.3 Interface avec M1

- `Arc<Mutex<Box<dyn ConsensusDetector>>>` injecté dans `SwarmController`.
- M1 appelle `ingest` une fois par round.
- M1 lit `verdict.is_terminal()` pour décider de sortir de la boucle.
- M1 lit `verdict.should_rollback()` pour déclencher M7.

### 13.4 Interface avec M7 & M8

M6 n'appelle jamais M7/M8 directement — le controller orchestre en se basant
sur le verdict. M6 fournit la *donnée de décision*, pas l'action.

### 13.5 Interface avec M11

M11 sérialise :
- `ConsensusVerdict` (enum avec serde)
- `Signals` (émis via un hook optionnel `on_signals_computed`)
- `history.iter()` (via un snapshot API optionnel)

---

## 14. Plan de sprint M6

### 14.1 Sprint 5 — MVP (1 semaine)

- **J1** : Scaffolding (`consensus/`), `ConsensusVerdict`, `ConsensusConfig`.
- **J2** : `ConvergenceHistory` + tests.
- **J3** : Signaux S1-S4 (scalar) + tests.
- **J4** : Signaux S5 Theil-Sen + S3 centroid drift + tests.
- **J4** : `ConfidenceScorer` + `SignalWeights` validation.
- **J5** : `MultiCriteriaDetector` — ingest flow, hystérésis, divergence, stall.
- **J5** : Tests d'intégration M5 → M6 + property tests.

**Livrables** :
- 80+ tests M6 verts
- clippy pédantique clean
- Doc générée
- `efe_core::consensus::MultiCriteriaDetector` utilisable

### 14.2 Sprint 6 — Tuning & robustness (0.5 semaine)

- EWMA alternative pour variance trend (streaming).
- Adaptation des weights en ligne (exponentiel smoothing).
- Benchmark criterion par signal.
- Test de stress : 1000 rounds synthétiques sans `Converged` → `Stalled`.

### 14.3 Sprint 7 — Bayesian (futur, hors MVP)

- `BayesianDetector` : maintient une distribution sur la "convergence
  probability", update via likelihood on each round. Utile quand les données
  sont bruitées ou les signaux conflictuels.

---

## 15. Critères d'acceptation

### 15.1 Fonctionnels

- [ ] Un `MultiCriteriaDetector` ingère 16 `FusionStats` consécutifs et
  rend un verdict cohérent (Converged si signaux bons, Continue sinon).
- [ ] Une anomalie (`has_nan = true`) est détectée en **1 ingest**.
- [ ] Hystérésis `K=2` est respectée (pas de `Converged` prématuré).
- [ ] `reset()` remet tout à l'état initial ; `ingest` après `reset` se comporte
  comme un nouveau detector.
- [ ] Serialization serde round-trips sur `ConsensusConfig` et `ConsensusVerdict`.

### 15.2 Qualité

- [ ] `cargo fmt --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` pédantique clean.
- [ ] `cargo doc --no-deps --workspace` sans warnings.
- [ ] Couverture de tests ≥ 90 % sur `consensus/`.
- [ ] **Zéro** `unsafe` dans M6 (math pur host).
- [ ] Tous les traits et types publics documentés.

### 15.3 Performance

- [ ] `ingest` < 100 µs pour `W=6, D=2048`.
- [ ] Footprint memory < 20 KB par detector.
- [ ] Zéro allocation hot-path (après le clone centroid, qui est inévitable).

### 15.4 Observabilité

- [ ] `ConsensusVerdict` sérialisable vers JSON.
- [ ] `Signals` exposés pour M11 (debug view).
- [ ] Tracing spans `consensus.ingest`, `consensus.signals`.

---

## 16. Risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Seuils par défaut mal calibrés | Élevée | Moyen | Tuning sur benchmarks synthétiques + corpus de runs. Config externe. |
| R2 | Faux `Converged` sur plateau | Moyen | Moyen | Signal S5 (trend) + hystérésis K=2. |
| R3 | Faux `Diverged` sur spike ponctuel | Moyen | Élevé | Theil-Sen (robuste aux outliers) pour S5. |
| R4 | Entropy signal trompeur si N varie | Moyen | Moyen | Normalisation par ln(N_active). |
| R5 | Floating-point NaN dans les signaux | Bas | Moyen | Check `is_finite()` + failsafe à 0. |
| R6 | History non synchronisée avec rounds (skip) | Bas | Élevé | Validation de monotonicity + AnomalyCode::NonMonotonicRound. |
| R7 | Detector partagé entre threads (bug config) | Bas | Élevé | Mutex enforcé + doc. |
| R8 | Poids qui ne somment pas à 1 | Moyen | Bas | Validation à construction, rejet sinon. |
| R9 | Campagne sans convergence ni stall | Bas | Moyen | Controller impose `max_rounds` — fallback à `Continue → abort`. |
| R10 | Serde breaking change sur variants | Bas | Moyen | `#[non_exhaustive]` sur tous les enums publics. |

---

## 17. Annexes

### 17.1 Glossaire

| Terme | Définition |
|---|---|
| **Consensus** | État où le swarm a trouvé un accord mesurable sur un espace latent stable. |
| **Signal** | Fonction `s_i(history) ∈ [0, 1]` qui mesure une facette de la convergence. |
| **Confidence** | Somme pondérée des signaux, `∈ [0, 1]`. |
| **Hystérésis** | Requérir plusieurs rounds consécutifs d'accord pour verdict terminal. |
| **Stall** | Plateau : confidence flat sous le seuil — pas de progrès. |
| **Divergence** | Situation où la variance explose ou la confidence décroit gravement. |
| **Anomaly** | NaN/Inf détecté, incohérence structurelle. |
| **Theil-Sen** | Régresseur robuste : médiane des pentes deux-à-deux. |
| **Window** | Fenêtre glissante des `W` derniers rounds. |
| **Monotonic round** | `r_n > r_{n-1}` sur l'historique. |

### 17.2 Pseudo-code complet `MultiCriteriaDetector::ingest`

```
fn ingest(self, stats):
    validate_stats_shape(stats)

    if last_round is set and stats.round < last_round:
        return set_verdict(Anomaly(NonMonotonicRound))
    last_round = stats.round

    if stats.has_numerical_issues():
        history.push(StatsRecord::from(stats))  # still record
        return set_verdict(Anomaly(NanInf))

    prev_centroid = history.last().centroid if history.len() > 0 else None
    history.push(StatsRecord::from(stats))

    signals = compute_signals(history, config, stats, prev_centroid)
    confidence = scorer.score(signals)
    last_confidence = confidence
    history.update_last_confidence(confidence)

    if detect_divergence(stats, confidence) is Some(v):
        return set_verdict(v)

    if confidence >= config.confidence_threshold:
        consecutive_high += 1
        if consecutive_high >= config.hysteresis_rounds:
            return set_verdict(Converged(confidence))
    else:
        consecutive_high = 0

    if detect_stall() is Some(v):
        return set_verdict(v)

    return set_verdict(Continue)
```

### 17.3 Exemple numérique

**Setup** : `W=6, threshold=0.80, K=2`.

**Rounds 1-8** : variances [0.5, 0.3, 0.2, 0.12, 0.06, 0.03, 0.02, 0.015].
Entropies centrées descendent, outliers = 0.02 constant, centroids stables.

```
r   variance  entropy  centroid-drift  s1      s2    s3    s4   s5    s6    conf    verdict
1   0.50      3.5      N/A              0.0     0.3   0.0   0.6  0.5   1.0  ~0.35   Continue
2   0.30      3.0      0.020            0.0     0.4   0.5   0.6  0.6   1.0  ~0.42   Continue
3   0.20      2.5      0.018            0.0     0.5   0.6   0.6  0.7   1.0  ~0.48   Continue
4   0.12      2.0      0.015            0.0     0.6   0.7   0.6  0.75  1.0  ~0.54   Continue
5   0.06      1.5      0.010            0.4     0.7   0.8   0.6  0.8   1.0  ~0.67   Continue
6   0.03      1.0      0.008            0.7     0.8   0.85  0.6  0.85  1.0  ~0.78   Continue (conf < 0.80)
7   0.02      0.8      0.005            0.8     0.85  0.90  0.6  0.88  1.0  ~0.82   Continue (K=1)
8   0.015     0.6      0.003            0.85    0.9   0.95  0.6  0.90  1.0  ~0.85   Converged (K=2) ✓
```

### 17.4 Invariants formels

- `∀stats : ingest(stats) ⇒ verdict() ∈ {Continue, Converged, Diverged, Stalled, Anomaly}`
- `∀history h : confidence(h) ∈ [0, 1]`
- `stats.has_nan ∨ stats.has_inf ⇒ verdict = Anomaly` *en 1 ingest*
- `history.len() < hysteresis_rounds ⇒ ¬Converged`
- `reset() ⇒ verdict = Continue ∧ confidence = 0 ∧ history.len() = 0`

### 17.5 Mapping metrics (pour M11)

| Verdict variant | Label de metric |
|---|---|
| `Continue` | `consensus_verdict="continue"` |
| `Converged { confidence }` | `consensus_verdict="converged"`, histogram `consensus_confidence` |
| `Diverged { reason: VarianceExplosion }` | `consensus_verdict="diverged", reason="variance_explosion"` |
| `Diverged { reason: VarianceGrowing }` | `consensus_verdict="diverged", reason="variance_growing"` |
| `Diverged { reason: ConfidenceDecay }` | `consensus_verdict="diverged", reason="confidence_decay"` |
| `Stalled { rounds_flat }` | `consensus_verdict="stalled"`, histogram `consensus_stall_rounds` |
| `Anomaly { code }` | `consensus_verdict="anomaly", code=...` |

### 17.6 Diagramme d'état détaillé

```
  ┌───────────────┐
  │    Reset      │ ← externally triggered
  └───────┬───────┘
          ▼
  ┌───────────────┐
  │   Observing   │  confidence < threshold
  └───┬───────────┘
      │                ingest(stats)
      ├─────────────────────────┐
      ▼                         │
  signals computed              │
      │                         │
      ▼                         │
  ┌──────────────────────┐      │
  │ stats.has_anomaly?   │─Yes→ [Anomaly (terminal)]
  └──────────────────────┘      │
      │ No                      │
      ▼                         │
  ┌──────────────────────┐      │
  │ variance explode?    │─Yes→ [Diverged (terminal)]
  └──────────────────────┘      │
      │ No                      │
      ▼                         │
  ┌──────────────────────┐      │
  │ confidence ≥ th?     │      │
  └──────────────────────┘      │
       │     │                  │
       │Yes  │No                │
       ▼     ▼                  │
  consec++  consec=0            │
       │     │                  │
       │     ▼                  │
       │  stalled?─Yes→ [Stalled (terminal)]
       │     │No                │
       │     └──────────────────┘
       ▼
  consec ≥ K?─No→ [Observing]
       │Yes
       ▼
  [Converged (terminal)]
```

### 17.7 Checklist d'audit post-implémentation

- [ ] Chaque signal `s_i` a un test qui isole son comportement.
- [ ] `ConsensusConfig::default()` passe `validate()`.
- [ ] Weights custom rejetés si sum != 1 (tolérance 1e-6).
- [ ] `ingest` avec round régressif retourne `Anomaly`.
- [ ] `history.is_full()` est vérifié avant `detect_stall`.
- [ ] Le detector est `Send + Sync`.
- [ ] Au moins un test property-based par signal.
- [ ] `Signals::all_finite()` renvoie `false` sur NaN/Inf.
- [ ] `ConsensusVerdict::is_terminal()` est cohérent avec les variants.
- [ ] Serde round-trips sur tous les types publics.

---

**Fin du document M6 ARCHITECTURE.md** — prêt pour implémentation sprint 5.
