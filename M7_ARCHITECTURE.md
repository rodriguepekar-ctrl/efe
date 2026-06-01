# M7 — Collapse / Decoherence Engine
## Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Module** : `efe_core::collapse`
**Sprints ciblés** : Sprint 6 (MVP 5 stratégies) → Sprint 7 (tuning adaptatif + escalade)
**Dépend de** : M1 (Core), M4 (ELS), M5 (ReplayBuffer + FusionStrategy), M6 (ConsensusVerdict)
**Consommé par** : M1 controller (rollback/recovery loop)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP](#2-scope-mvp)
3. [Théorie : 5 stratégies de collapse](#3-théorie--5-stratégies-de-collapse)
4. [Vue d'ensemble architecturale](#4-vue-densemble-architecturale)
5. [Traits et abstractions](#5-traits-et-abstractions)
6. [Structures de données](#6-structures-de-données)
7. [Flux d'une collapse](#7-flux-dune-collapse)
8. [Algorithmes détaillés](#8-algorithmes-détaillés)
9. [Machine à états & escalade](#9-machine-à-états--escalade)
10. [Performance et budget](#10-performance-et-budget)
11. [Interface publique](#11-interface-publique)
12. [Stratégie de test](#12-stratégie-de-test)
13. [Dépendances inter-modules](#13-dépendances-inter-modules)
14. [Plan de sprint M7](#14-plan-de-sprint-m7)
15. [Critères d'acceptation](#15-critères-dacceptation)
16. [Risques et mitigations](#16-risques-et-mitigations)
17. [Annexes](#17-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M7 est le **mécanisme de récupération** du pipeline EFE. Quand M6 rend un
verdict non-convergent (`Diverged`, `Anomaly`, ou un `Stalled` que le
controller décide de ne pas accepter), M7 choisit une **action de
remédiation** pour :

1. **Préserver la cohérence numérique** — éliminer tout NaN/Inf avant
   qu'il ne se propage à M8 (output projection) ou corrompt M11
   (telemetry).
2. **Tenter une récupération bon marché** — rollback, injection de bruit,
   désactivation d'outliers — avant d'abandonner le swarm.
3. **Déclarer l'échec si inévitable** — rendre un `CollapseOutcome` avec
   `success=false` et des métriques détaillées, pour que M1 décide de
   respawn complet (via M3) ou d'abort.

Le nom "collapse" vient de la physique quantique : l'état superposé (le
swarm incertain) est projeté sur un état unique et bien défini, même si
ce n'est pas celui qu'on voulait. Une fois collapsé, on peut réessayer.

### 1.2 Contraintes et garanties

- **Transactionnel** : `collapse.handle(...)` retourne un résultat
  atomique. Soit l'ELS est dans un nouvel état valide, soit inchangé. Pas
  d'état intermédiaire observable par M1/M11.
- **Déterministe conditionnel** : même state initial + même `seed` →
  même collapse. Sauf `NoiseInjection` qui utilise un PRNG seedé.
- **Borné en temps** : toute stratégie a un budget ≤ 50 ms. Une
  stratégie qui overtime est interrompue et marquée comme échec.
- **Observable** : chaque collapse émet un `CollapseEvent` consommable
  par M11 (timestamp, trigger, action, outcome).

### 1.3 Position dans le pipeline

```
  ┌─────────────────────────────────────────────────────────┐
  │                Controller (M1)                           │
  └──┬──────────────┬──────────────┬──────────────┬──────────┘
     │              │              │              │
   spawn()      fuse(r)       check(r)       collapse()
     │              │              │              │
     ▼              ▼              ▼              ▼
  ┌─────┐       ┌──────┐     ┌──────────┐     ┌──────┐
  │ M3  │──▶    │  M5  │──▶  │   M6     │──▶  │  M7  │
  │Gene-│       │Fusion│     │Consensus │     │Coll- │
  │sis  │       └──┬───┘     └────┬─────┘     │apse  │
  └─────┘          │              │           └──┬───┘
                   │              │              │
                   │              ▼              │
                   │         Verdict             │
                   │         (Diverged,          │
                   │          Anomaly, …)        │
                   │                             │
                   ▼                             ▼
              ┌────────────────────────────────────┐
              │      M4 — ELS (shared state)        │
              │   collapse reads + writes latent    │
              └────────────────────────────────────┘
                       ▲
                       │ replay buffer access
                       │
                  ┌────┴──────┐
                  │  M5       │
                  │ ReplayBuf │
                  └───────────┘
```

### 1.4 Garanties offertes aux modules avals

1. **Pas d'état NaN/Inf après `handle()` réussi** — post-condition stricte.
2. **Variance post-collapse bornée** : `variance_after ≤ variance_config_max`
   (typiquement `10× variance_threshold` de M6).
3. **Métriques ELS cohérentes** — `compute_stats(&els, &arena).all_finite()`
   vaut `true` après un collapse réussi.
4. **Trigger preservation** — le `CollapseOutcome` contient l'input trigger
   pour traçabilité M11.

---

## 2. Scope MVP

### 2.1 IN

| Fonctionnalité | Justification MVP |
|---|---|
| Trait `CollapseStrategy` (DI) | Permet l'injection de stratégies custom sans toucher M1. |
| 5 stratégies MVP | Couverture large des scénarios de failure. |
| `CollapsePolicy` avec escalade ordonnée | Essai progressif : soft → hard → abort. |
| `CollapseEngine` orchestration + history | Permet à M1 d'interagir avec un seul point d'entrée. |
| `CollapseOutcome` discriminé | M1 branche sur succès/échec, M11 sérialise. |
| Integration avec `ReplayableFusion` | Réutilise le replay buffer déjà en place dans M5. |
| PRNG seedé (`NoiseInjection`) | Déterministe en CI, randomisé en prod via `SwarmId`. |
| `CollapseEvent` sérialisable | Observabilité immédiate. |
| Circuit breaker (max N collapses / swarm) | Empêche les boucles infinies collapse ↔ fusion. |

### 2.2 OUT (reporté Sprint 7+)

| Fonctionnalité | Raison du report |
|---|---|
| Meta-learning pour choisir la stratégie | Nécessite dataset de runs. Sprint 8. |
| Collapse asynchrone (pendant la fusion) | Complexe, incompatible avec ELS `&mut self`. v2. |
| Collapse partiel par sous-population | Nécessite M9 (multi-swarm). Sprint 9. |
| Snapshot diff-compression | Optim mémoire, hors scope MVP. Sprint 7. |
| Collapse distribué cross-node | Hors scope MVP mono-node. v2. |

### 2.3 Principe "escalade ordonnée"

M7 ne choisit **pas** une stratégie au hasard. Il applique une
**escalade stricte** :

```
Level 1 — SoftRollback           (cheapest, non-destructive)
   ↓ if fails / inapplicable
Level 2 — NoiseInjection         (small perturbation, may escape local min)
   ↓ if fails
Level 3 — CentroidProjection     (drastic: variance → 0)
   ↓ if fails
Level 4 — PartialRespawn         (drops outlier agents)
   ↓ if fails
Level 5 — FullAbort              (terminal: M1 must respawn via M3)
```

Les niveaux 1-4 sont **récupératifs** (reusable swarm). Le niveau 5 est
terminal. Le choix n'est pas arbitraire : chaque niveau altère plus
profondément le swarm mais offre plus de chances de récupération.

---

## 3. Théorie : 5 stratégies de collapse

### 3.1 Stratégie 1 — `SoftRollback`

**But** : restaurer le latent à un round précédent connu sain.

**Préconditions** :
- Le `ReplayBuffer` de M5 contient au moins 1 snapshot valide.
- Le snapshot le plus récent n'est pas lui-même corrompu (check post-restore).

**Algorithme** :
```
1. target_round = replay.latest_valid_round()
2. replay.restore(target_round, &mut els)
3. validate_els_post_restore(&els) → Ok / Err
4. if Err, try target_round - 1, etc.
```

**Coût** : O(N·D·2) — copie de `N*D*2` bytes depuis le snapshot. ≤ 1 ms.

**Quand l'appliquer** :
- `Anomaly::NanInf` — rollback au dernier round sans anomalie.
- `Divergence::VarianceExplosion` — restaurer avant l'explosion.
- `Divergence::ConfidenceDecay` — restaurer avant le pic.

**Quand NON l'appliquer** :
- Pas de snapshot disponible.
- Tous les snapshots sont corrompus (cas pathologique).
- `Divergence::VarianceGrowing` : les snapshots anciens sont *aussi*
  sur la trajectoire croissante, rollback ne résout rien.

### 3.2 Stratégie 2 — `NoiseInjection`

**But** : sortir d'un minimum local (stall) ou d'un état pathologique
en perturbant le latent.

**Algorithme** :
```
for each active agent i:
    for each dim d:
        latent[i,d] += bf16( sigma * randn(rng) )
        latent[i,d] = clip(latent[i,d], -clip_max, clip_max)
```

**Propriétés** :
- Préserve en moyenne le centroïde (bruit de moyenne 0).
- Augmente la variance de ~`D * sigma²`.
- `sigma` configurable, typique `0.05`.

**Coût** : O(N·D) — ~1 ms pour N=128, D=2048.

**Quand l'appliquer** :
- `Stalled` — le swarm est bloqué, on le secoue.
- `Divergence::ConfidenceDecay` après un `SoftRollback` échoué.

**Risques** :
- Bruit trop fort → variance explose à nouveau.
- Bruit trop faible → pas d'effet.

### 3.3 Stratégie 3 — `CentroidProjection`

**But** : réinitialiser "doucement" le swarm en projetant chaque ligne
sur le centroïde actuel.

**Algorithme** :
```
mu = compute_centroid(latent, mask)
for each active agent i:
    latent[i] = mu + tiny_noise(sigma=0.01)
```

**Effet** : variance ≈ 0 après projection. Le swarm "redémarre" depuis
le centre de masse, avec une légère dispersion pour éviter le point
fixe absolu.

**Coût** : O(N·D) — ~1 ms.

**Quand l'appliquer** :
- `Divergence::VarianceExplosion` quand rollback impossible (replay vide).
- Après échec de `NoiseInjection` et `SoftRollback`.

**Risques** :
- Perte d'information : toute la diversité du swarm est perdue.
- Si le centroïde lui-même est pathologique, la projection propage le
  problème.

### 3.4 Stratégie 4 — `PartialRespawn`

**But** : désactiver les agents "outliers" (identifiés par
`outlier_ratio` élevé) et continuer avec le reste.

**Algorithme** :
```
stats = compute_stats(&els, &arena)
for each active agent i:
    d = distance(latent[i], stats.centroid)
    if d > 2 * stats.gradient_norm:
        mask[i] = 0   # deactivate
```

**Préconditions** :
- Au moins `min_active_after` agents restent actifs (default: 50% de N).

**Effet** : population réduite, mais le "cœur" du swarm est préservé.
Variance après = ~50-70% de variance avant.

**Coût** : O(N·D) pour le compute_stats + O(N·D) pour le scan. ~2 ms.

**Quand l'appliquer** :
- `Divergence::VarianceGrowing` où les outliers sont la cause.
- Après échec de `CentroidProjection`.

**Risques** :
- Peut désactiver trop d'agents → fonctionne plus.
- N'aide pas si les outliers sont une minorité et la vraie cause est
  systémique (drift global).

### 3.5 Stratégie 5 — `FullAbort`

**But** : déclarer le swarm irrécupérable.

**Algorithme** :
```
return CollapseOutcome {
    action_taken: FullAbort,
    success: false,
    new_round: None,
    stats_after: None,
    recovery_confidence: 0.0,
    warnings: vec![CollapseWarning::Abandoned],
}
```

**Effet** : aucun sur l'ELS. Le controller reçoit l'outcome et doit :
- Soit respawn via M3 avec le même `SwarmSpec`.
- Soit remonter l'erreur au client (timeout, budget).

**Coût** : O(1).

**Quand l'appliquer** :
- Toutes les stratégies précédentes ont échoué.
- Compteur de collapses pour ce swarm > `max_collapses_per_swarm`.

### 3.6 Synthèse : matrice trigger × stratégie

| Trigger M6 | L1 SoftRollback | L2 NoiseInject | L3 CentProj | L4 PartialResp | L5 Abort |
|---|---|---|---|---|---|
| `Anomaly::NanInf` | ✅ | ⚠️ | ⚠️ | ⚠️ | ✅ |
| `Anomaly::CentroidDimMismatch` | ❌ | ❌ | ❌ | ❌ | ✅ |
| `Anomaly::NonMonotonicRound` | ❌ | ❌ | ❌ | ❌ | ✅ (via controller bug) |
| `Div::VarianceExplosion` | ✅ | ⚠️ | ✅ | ⚠️ | ✅ |
| `Div::VarianceGrowing` | ⚠️ | ✅ | ⚠️ | ✅ | ✅ |
| `Div::ConfidenceDecay` | ✅ | ✅ | ⚠️ | ⚠️ | ✅ |
| `Stalled` | ❌ | ✅ | ⚠️ | ⚠️ | ✅ |

**Légende** : ✅ = stratégie préférée, ⚠️ = fallback acceptable,
❌ = inapplicable.

---

## 4. Vue d'ensemble architecturale

### 4.1 Disposition

```
efe-core/src/collapse/
├── mod.rs                    # Facade + re-exports
├── config.rs                 # CollapseConfig
├── trigger.rs                # CollapseTrigger + From<ConsensusVerdict>
├── outcome.rs                # CollapseOutcome + CollapseWarning + CollapseAction
├── event.rs                  # CollapseEvent (telemetry)
├── context.rs                # CollapseContext<'a> (handle to ELS/arena/fusion)
├── strategy.rs               # CollapseStrategy trait
├── policy.rs                 # CollapsePolicy
├── engine.rs                 # CollapseEngine (orchestration)
├── validator.rs              # Post-collapse invariant checks
└── strategies/
    ├── mod.rs                # Re-exports
    ├── rollback.rs           # SoftRollbackStrategy
    ├── noise.rs              # NoiseInjectionStrategy
    ├── centroid_project.rs   # CentroidProjectionStrategy
    ├── partial_respawn.rs    # PartialRespawnStrategy
    ├── abort.rs              # FullAbortStrategy (terminal)
    └── stub.rs               # StubStrategy (tests)
```

### 4.2 Diagramme de classes

```
                  ┌──────────────────────────────┐
                  │     CollapseStrategy (trait) │
                  │  async fn apply(ctx) → Out   │
                  │  fn applicable_to(trigger)   │
                  └──────────┬───────────────────┘
                             │ impl
        ┌──────────┬─────────┼──────────┬──────────┬──────────┐
        ▼          ▼         ▼          ▼          ▼          ▼
  ┌─────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌─────────┐
  │ Soft    │ │ Noise  │ │Centroid│ │Partial │ │ Full   │ │  Stub   │
  │Rollback │ │Injection│ │Project │ │Respawn │ │ Abort  │ │ (tests) │
  └─────────┘ └────────┘ └────────┘ └────────┘ └────────┘ └─────────┘

                  ┌──────────────────────────────┐
                  │     CollapseEngine            │
                  │   holds: CollapsePolicy       │
                  │   holds: history Vec<Event>   │
                  │   holds: per-swarm counters   │
                  └──────────┬───────────────────┘
                             │ uses
                             ▼
                  ┌──────────────────────────────┐
                  │ CollapsePolicy               │
                  │  escalation_order: Vec<Arc<  │
                  │      dyn CollapseStrategy>>  │
                  │  max_retries                 │
                  └──────────────────────────────┘
```

### 4.3 Pipeline d'une collapse

```
     ┌────────────────────────────────────┐
     │   engine.handle(verdict, els, …)    │
     └────────────────┬───────────────────┘
                      ▼
           ┌───────────────────────┐
           │  trigger = from(verdict)│
           └──────────┬────────────┘
                      ▼
           ┌───────────────────────┐
           │ check circuit breaker  │
           │  (max_collapses count) │
           └──────────┬────────────┘
                      │
           ┌──────────┴────────────┐
           │        escalation      │
           ▼                        ▼
     for strat in policy.escalation_order:
         if strat.applicable_to(trigger):
             out = strat.apply(ctx).await
             emit CollapseEvent
             if out.success:
                 return out
     return FullAbort outcome
```

---

## 5. Traits et abstractions

### 5.1 `CollapseStrategy` — contrat

```rust
/// Apply a single recovery strategy to an ELS.
///
/// # Contract
/// - Must be idempotent on a given input state.
/// - Must respect the wallclock budget in `CollapseContext::deadline`.
/// - Must leave the ELS in a finite state on success (no NaN/Inf).
/// - On failure, the ELS may be left partially modified — the
///   engine is responsible for recovery via the next strategy.
#[async_trait::async_trait]
pub trait CollapseStrategy: Send + Sync + std::fmt::Debug {
    /// Execute the strategy. Returns `Ok(outcome)` whether or not
    /// the strategy actually fixed the swarm — the `success` field
    /// of `CollapseOutcome` reports that.
    ///
    /// # Errors
    /// - `EfeError::Internal` on contract violations (e.g. ELS is stub).
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome>;

    /// Cheap predicate: can this strategy reasonably handle the given trigger?
    /// Does NOT require taking a mutable borrow of the ELS.
    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool;

    /// Human-readable name, stable across MINOR bumps.
    fn name(&self) -> &'static str;
}
```

### 5.2 `CollapseEngine` — orchestration

```rust
pub struct CollapseEngine {
    policy: CollapsePolicy,
    /// Telemetry history. Bounded; oldest events evicted.
    events: VecDeque<CollapseEvent>,
    events_capacity: usize,
    /// Per-swarm collapse counter (swarm_id → count).
    /// Used for the circuit breaker.
    collapse_counts: HashMap<SwarmId, u32>,
    config: CollapseConfig,
}

impl CollapseEngine {
    pub fn new(policy: CollapsePolicy, config: CollapseConfig) -> Result<Self>;

    /// Top-level entry point. Called by M1.
    pub async fn handle(
        &mut self,
        verdict: &ConsensusVerdict,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        fusion: Arc<dyn ReplayableFusion>,
        round: u32,
    ) -> Result<CollapseOutcome>;

    pub fn events(&self) -> impl Iterator<Item = &CollapseEvent>;
    pub fn collapse_count(&self, swarm_id: SwarmId) -> u32;
    pub fn reset_swarm_counter(&mut self, swarm_id: SwarmId);
}
```

### 5.3 `CollapsePolicy` — escalation ordering

```rust
/// Ordered list of strategies to try, and retry rules.
pub struct CollapsePolicy {
    pub escalation_order: Vec<Arc<dyn CollapseStrategy>>,
    pub max_retries_per_call: u32,
}

impl CollapsePolicy {
    pub fn default_mvp() -> Self;
    pub fn custom(strategies: Vec<Arc<dyn CollapseStrategy>>) -> Self;
}
```

### 5.4 `CollapseContext<'a>` — handle for strategies

```rust
pub struct CollapseContext<'a> {
    pub els: &'a mut EpistemicLatentSpace,
    pub arena: &'a Arena,
    pub fusion: Arc<dyn ReplayableFusion>,
    pub round: u32,
    pub trigger: CollapseTrigger,
    pub verdict: ConsensusVerdict,
    /// Deterministic PRNG seeded from swarm_id + round.
    pub rng: Pcg64,
    /// Hard wallclock deadline (instant).
    pub deadline: Instant,
    /// Shared config read-only.
    pub config: &'a CollapseConfig,
}
```

---

## 6. Structures de données

### 6.1 `CollapseConfig`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct CollapseConfig {
    /// Max collapses per swarm before forced FullAbort.
    pub max_collapses_per_swarm: u32,          // default: 5

    /// Wall-clock budget for a single collapse call (ms).
    pub deadline_ms: u32,                       // default: 50

    /// Noise injection σ (if L2 strategy is used).
    pub noise_sigma: f32,                       // default: 0.05

    /// Clip range after collapse — identical to M5 clip_max.
    pub clip_max: f32,                          // default: 32.0

    /// Centroid projection: small-noise σ to avoid exact fixed point.
    pub centroid_project_jitter: f32,           // default: 0.01

    /// Partial respawn: z-score threshold for outliers.
    pub outlier_z_threshold: f32,               // default: 2.0

    /// Partial respawn: minimum fraction of agents that must stay active.
    pub min_active_fraction: f32,               // default: 0.5

    /// Post-collapse variance cap — if exceeded, treat as failure.
    pub post_variance_max: f32,                 // default: 10.0

    /// Events history capacity.
    pub events_capacity: u32,                   // default: 64

    /// Use deterministic seed (CI mode) — skip hash-randomization.
    pub deterministic_seed: bool,               // default: false
}
```

Validation :
- `max_collapses_per_swarm ≥ 1`
- `deadline_ms ≥ 1`
- `noise_sigma > 0.0`, finite
- `clip_max > 0.0`
- `outlier_z_threshold > 0.0`
- `min_active_fraction ∈ (0, 1]`
- `post_variance_max > 0.0`
- `events_capacity ≥ 1`

### 6.2 `CollapseTrigger`

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseTrigger {
    FromDivergence {
        reason: DivergenceReason,
        round: u32,
    },
    FromAnomaly {
        code: AnomalyCode,
        round: u32,
    },
    FromStall {
        rounds_flat: u32,
        confidence: f32,
        round: u32,
    },
    /// Manually invoked by the controller (e.g. operator intervention).
    Manual {
        reason: String,
        round: u32,
    },
}

impl CollapseTrigger {
    pub fn from_verdict(verdict: &ConsensusVerdict, round: u32) -> Option<Self>;
    pub fn round(&self) -> u32;
    pub fn label(&self) -> &'static str; // "divergence" | "anomaly" | …
}
```

### 6.3 `CollapseAction`

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseAction {
    SoftRollback {
        /// Round restored.
        to_round: u32,
    },
    NoiseInjection {
        /// Sigma used.
        sigma: f32,
        /// PRNG seed for reproducibility.
        seed: u64,
    },
    CentroidProjection {
        /// Jitter sigma.
        jitter: f32,
    },
    PartialRespawn {
        /// Agents deactivated.
        deactivated: u32,
        /// Agents remaining active.
        remaining: u32,
    },
    FullAbort,
    /// No strategy was applicable — engine gave up.
    NoOp,
}
```

### 6.4 `CollapseOutcome`

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CollapseOutcome {
    pub action_taken: CollapseAction,
    pub success: bool,
    pub trigger: CollapseTrigger,
    /// Round index AFTER the collapse — equals trigger.round() if no
    /// rollback, equals target_round for SoftRollback.
    pub new_round: Option<u32>,
    /// Variance after the collapse, if ELS is in a finite state.
    pub variance_after: Option<f32>,
    /// Confidence estimate that subsequent fusion will converge.
    /// Heuristic in MVP; ML model in Sprint 7.
    pub recovery_confidence: f32,
    /// Non-fatal issues.
    pub warnings: Vec<CollapseWarning>,
    /// Time spent (μs).
    pub duration_us: u32,
    /// Name of the strategy that produced this outcome.
    pub strategy_name: String,
}

impl CollapseOutcome {
    pub fn is_recoverable(&self) -> bool {
        self.success && !matches!(self.action_taken, CollapseAction::FullAbort)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.action_taken, CollapseAction::FullAbort)
            || (!self.success && self.warnings.iter().any(|w| w.is_fatal()))
    }
}
```

### 6.5 `CollapseWarning`

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CollapseWarning {
    /// Replay buffer had no valid snapshot.
    ReplayBufferEmpty,
    /// All replay snapshots were themselves corrupt.
    ReplayAllCorrupt,
    /// Noise injection caused variance to still exceed threshold.
    NoiseIneffective { variance_after: f32 },
    /// Partial respawn would leave too few active agents.
    PartialRespawnBelowMinimum { would_deactivate: u32, min_required: u32 },
    /// Collapse deadline exceeded.
    DeadlineExceeded { budget_us: u32, actual_us: u32 },
    /// Circuit breaker fired — max collapses reached.
    CircuitBreaker { swarm_collapse_count: u32 },
    /// Swarm abandoned after exhausting all strategies.
    Abandoned,
    /// ELS post-condition failed: still has NaN/Inf.
    PostConditionFailed,
}

impl CollapseWarning {
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::CircuitBreaker { .. } | Self::Abandoned | Self::PostConditionFailed
        )
    }
}
```

### 6.6 `CollapseEvent` (telemetry)

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollapseEvent {
    pub timestamp_nanos: u64,
    pub swarm_id: SwarmId,
    pub trigger: CollapseTrigger,
    pub outcome: CollapseOutcome,
    pub collapse_index: u32,   // 1-based, per swarm
}
```

---

## 7. Flux d'une collapse

### 7.1 Séquence chronologique

```
T=0            M1 detects Diverged → calls engine.handle(verdict, els, …)
T=~5μs         trigger = from_verdict(verdict) ; Some(…) or None (non-collapse verdict)
T=~10μs        check collapse_counts[swarm_id] against max_collapses_per_swarm
               if exceeded → return FullAbort with CircuitBreaker warning
T=~15μs        increment collapse_counts[swarm_id]
T=~20μs        for each strategy in policy.escalation_order:
                  if !strat.applicable_to(&trigger) → skip
                  ctx = CollapseContext { els, arena, fusion, rng, deadline, … }
                  outcome = strat.apply(&mut ctx).await?
                  emit CollapseEvent to history
                  if outcome.is_recoverable():
                      return outcome
T=end          if loop exhausted → return FullAbort outcome
```

### 7.2 Invariants de sortie

- Si `success = true` : `compute_stats(&els, &arena).all_finite()` est `true`.
- Si `success = true` : `stats.variance <= config.post_variance_max`.
- `outcome.duration_us <= config.deadline_ms * 1000` (± scheduling jitter).
- `events.len() <= events_capacity` (ring).

### 7.3 Allocations

Une collapse ne doit **pas** allouer dans la hot-path :
- `SoftRollback` : uses ReplayBuffer's pre-allocated arena. No alloc.
- `NoiseInjection` : scratch vector `Vec<f32>` of size D. One alloc per call,
  could be pre-allocated in engine state.
- `CentroidProjection` : uses `stats.centroid` already in `ElsStats`.
- `PartialRespawn` : in-place mask update. No alloc.

Sprint 6 optimization : scratch pool dans `CollapseEngine`.

---

## 8. Algorithmes détaillés

### 8.1 `SoftRollbackStrategy`

```rust
impl CollapseStrategy for SoftRollbackStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let swarm_id = ctx.els.swarm_id();

        // Try the most recent snapshot first, then older ones.
        let mut tried_rounds: Vec<u32> = vec![];
        let target_rounds = ctx.fusion.available_replay_rounds(); // newest-first
        if target_rounds.is_empty() {
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: None,
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::ReplayBufferEmpty],
                duration_us: start.elapsed().as_micros() as u32,
                strategy_name: self.name().into(),
            });
        }

        for target in target_rounds {
            // Skip rounds that were themselves anomalous if we know about them
            // (Sprint 7+ will track per-snapshot anomaly status).
            ctx.fusion
                .restore_snapshot(target, ctx.els, ctx.arena)
                .await?;
            let stats = compute_stats(ctx.els, ctx.arena)?;
            if !stats.all_finite() {
                tried_rounds.push(target);
                continue;
            }
            if stats.variance > ctx.config.post_variance_max {
                tried_rounds.push(target);
                continue;
            }
            // Success.
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::SoftRollback { to_round: target },
                success: true,
                trigger: ctx.trigger.clone(),
                new_round: Some(target),
                variance_after: Some(stats.variance),
                recovery_confidence: estimate_recovery_confidence(&stats, ctx.config),
                warnings: vec![],
                duration_us: start.elapsed().as_micros() as u32,
                strategy_name: self.name().into(),
            });
        }

        // All snapshots corrupt or too-divergent.
        Ok(CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: false,
            trigger: ctx.trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::ReplayAllCorrupt],
            duration_us: start.elapsed().as_micros() as u32,
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        !matches!(
            trigger,
            CollapseTrigger::FromAnomaly {
                code: AnomalyCode::CentroidDimMismatch { .. }
                    | AnomalyCode::NonMonotonicRound { .. },
                ..
            }
        )
    }

    fn name(&self) -> &'static str { "soft-rollback" }
}
```

### 8.2 `NoiseInjectionStrategy`

```rust
impl CollapseStrategy for NoiseInjectionStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let sigma = ctx.config.noise_sigma;

        // Capture seed for reproducibility.
        let seed = ctx.rng.next_u64();
        let mut rng = Pcg64::seed_from_u64(seed);
        let normal = Normal::new(0.0, sigma).map_err(|_| {
            EfeError::Internal("noise_injection: failed to build normal dist")
        })?;

        {
            let mut split = ctx.els.split_mut(ctx.arena)?;
            let n = split.latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = split.latent.row_mut(i).expect("i < n");
                for r in row.iter_mut() {
                    let v = r.to_f32() + normal.sample(&mut rng);
                    let clipped = v.clamp(-ctx.config.clip_max, ctx.config.clip_max);
                    *r = bf16::from_f32(clipped);
                }
            }
        }

        let stats = compute_stats(ctx.els, ctx.arena)?;
        let success =
            stats.all_finite() && stats.variance <= ctx.config.post_variance_max;
        let mut warnings = vec![];
        if stats.variance > ctx.config.post_variance_max {
            warnings.push(CollapseWarning::NoiseIneffective {
                variance_after: stats.variance,
            });
        }

        Ok(CollapseOutcome {
            action_taken: CollapseAction::NoiseInjection { sigma, seed },
            success,
            trigger: ctx.trigger.clone(),
            new_round: Some(ctx.round),
            variance_after: Some(stats.variance),
            recovery_confidence: estimate_recovery_confidence(&stats, ctx.config),
            warnings,
            duration_us: start.elapsed().as_micros() as u32,
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        matches!(
            trigger,
            CollapseTrigger::FromStall { .. }
                | CollapseTrigger::FromDivergence {
                    reason: DivergenceReason::ConfidenceDecay { .. },
                    ..
                }
        )
    }

    fn name(&self) -> &'static str { "noise-injection" }
}
```

### 8.3 `CentroidProjectionStrategy`

```rust
impl CollapseStrategy for CentroidProjectionStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();

        let stats = compute_stats(ctx.els, ctx.arena)?;
        if !stats.all_finite() {
            // Centroid is itself corrupt — cannot project.
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: None,
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::PostConditionFailed],
                duration_us: start.elapsed().as_micros() as u32,
                strategy_name: self.name().into(),
            });
        }

        let centroid = stats.centroid.clone();
        let jitter = ctx.config.centroid_project_jitter;
        let seed = ctx.rng.next_u64();
        let mut rng = Pcg64::seed_from_u64(seed);
        let normal = Normal::new(0.0, jitter).unwrap();

        {
            let mut split = ctx.els.split_mut(ctx.arena)?;
            let n = split.latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = split.latent.row_mut(i).expect("i < n");
                for (r, c) in row.iter_mut().zip(centroid.iter()) {
                    let v = c + normal.sample(&mut rng);
                    *r = bf16::from_f32(v);
                }
            }
        }

        let stats_after = compute_stats(ctx.els, ctx.arena)?;
        Ok(CollapseOutcome {
            action_taken: CollapseAction::CentroidProjection { jitter },
            success: stats_after.all_finite(),
            trigger: ctx.trigger.clone(),
            new_round: Some(ctx.round),
            variance_after: Some(stats_after.variance),
            recovery_confidence: estimate_recovery_confidence(&stats_after, ctx.config),
            warnings: vec![],
            duration_us: start.elapsed().as_micros() as u32,
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, _trigger: &CollapseTrigger) -> bool {
        // Applicable whenever the centroid itself is still computable.
        // The pre-compute_stats check inside apply() guards against corrupt
        // centroids at runtime.
        true
    }

    fn name(&self) -> &'static str { "centroid-projection" }
}
```

### 8.4 `PartialRespawnStrategy`

```rust
impl CollapseStrategy for PartialRespawnStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let stats = compute_stats(ctx.els, ctx.arena)?;
        if !stats.all_finite() {
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: None,
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::PostConditionFailed],
                duration_us: start.elapsed().as_micros() as u32,
                strategy_name: self.name().into(),
            });
        }
        let threshold_sq = (ctx.config.outlier_z_threshold * stats.gradient_norm).powi(2);
        let centroid = stats.centroid.clone();
        let n_initial = stats.n_active;
        let min_active = (f32::from(n_initial as u16) * ctx.config.min_active_fraction) as u32;

        // First pass: identify outliers (without mutation).
        let mut to_deactivate: Vec<u32> = vec![];
        {
            let split = ctx.els.split_mut(ctx.arena)?;
            let n = split.latent.n_agents();
            for i in 0..n {
                if !split.mask.is_active(i) {
                    continue;
                }
                let row = split.latent.row(i).expect("i < n");
                let dsq = l2_distance_sq_mixed(row, &centroid);
                if dsq > threshold_sq {
                    to_deactivate.push(i);
                }
            }
        }

        // Check that we don't drop below the minimum.
        let remaining = n_initial.saturating_sub(to_deactivate.len() as u32);
        if remaining < min_active {
            return Ok(CollapseOutcome {
                action_taken: CollapseAction::NoOp,
                success: false,
                trigger: ctx.trigger.clone(),
                new_round: None,
                variance_after: Some(stats.variance),
                recovery_confidence: 0.0,
                warnings: vec![CollapseWarning::PartialRespawnBelowMinimum {
                    would_deactivate: to_deactivate.len() as u32,
                    min_required: min_active,
                }],
                duration_us: start.elapsed().as_micros() as u32,
                strategy_name: self.name().into(),
            });
        }

        // Second pass: actually deactivate.
        let deactivated = to_deactivate.len() as u32;
        {
            let mut mask = ctx.els.mask_view_mut(ctx.arena)?.ok_or(
                EfeError::Internal("partial_respawn: ELS missing mask")
            )?;
            for i in to_deactivate {
                mask.set(i, false)?;
            }
        }

        let stats_after = compute_stats(ctx.els, ctx.arena)?;
        Ok(CollapseOutcome {
            action_taken: CollapseAction::PartialRespawn {
                deactivated,
                remaining: stats_after.n_active,
            },
            success: stats_after.all_finite(),
            trigger: ctx.trigger.clone(),
            new_round: Some(ctx.round),
            variance_after: Some(stats_after.variance),
            recovery_confidence: estimate_recovery_confidence(&stats_after, ctx.config),
            warnings: vec![],
            duration_us: start.elapsed().as_micros() as u32,
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool {
        matches!(
            trigger,
            CollapseTrigger::FromDivergence {
                reason: DivergenceReason::VarianceGrowing { .. },
                ..
            }
        )
    }

    fn name(&self) -> &'static str { "partial-respawn" }
}
```

### 8.5 `FullAbortStrategy`

```rust
impl CollapseStrategy for FullAbortStrategy {
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome> {
        let start = Instant::now();
        Ok(CollapseOutcome {
            action_taken: CollapseAction::FullAbort,
            success: false,
            trigger: ctx.trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::Abandoned],
            duration_us: start.elapsed().as_micros() as u32,
            strategy_name: self.name().into(),
        })
    }

    fn applicable_to(&self, _trigger: &CollapseTrigger) -> bool { true }
    fn name(&self) -> &'static str { "full-abort" }
}
```

### 8.6 `CollapseEngine::handle`

```rust
impl CollapseEngine {
    pub async fn handle(
        &mut self,
        verdict: &ConsensusVerdict,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        fusion: Arc<dyn ReplayableFusion>,
        round: u32,
    ) -> Result<CollapseOutcome> {
        let start = Instant::now();

        // 1. Map verdict to trigger; non-collapse verdicts return NoOp.
        let trigger = match CollapseTrigger::from_verdict(verdict, round) {
            Some(t) => t,
            None => return Ok(self.no_op_outcome(round, start)),
        };

        // 2. Circuit breaker.
        let swarm_id = els.swarm_id();
        let count = self.collapse_counts.entry(swarm_id).or_insert(0);
        *count += 1;
        if *count > self.config.max_collapses_per_swarm {
            let outcome = self.abort_outcome(&trigger, start, *count);
            self.record_event(swarm_id, &trigger, &outcome);
            return Ok(outcome);
        }

        // 3. Seed deterministic RNG.
        let seed = if self.config.deterministic_seed {
            fnv1a64(&format!("{swarm_id:?}-{round}").as_bytes())
        } else {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
        };
        let rng = Pcg64::seed_from_u64(seed);

        let deadline = start + Duration::from_millis(u64::from(self.config.deadline_ms));

        // 4. Escalation loop.
        for strategy in &self.policy.escalation_order {
            if !strategy.applicable_to(&trigger) {
                continue;
            }
            let mut ctx = CollapseContext {
                els,
                arena,
                fusion: fusion.clone(),
                round,
                trigger: trigger.clone(),
                verdict: verdict.clone(),
                rng: rng.clone(),
                deadline,
                config: &self.config,
            };
            let outcome = strategy.apply(&mut ctx).await?;
            self.record_event(swarm_id, &trigger, &outcome);
            if outcome.is_recoverable() {
                return Ok(outcome);
            }
            if Instant::now() >= deadline {
                // Out of budget — abort immediately.
                let outcome = self.deadline_outcome(&trigger, start);
                self.record_event(swarm_id, &trigger, &outcome);
                return Ok(outcome);
            }
        }

        // 5. All strategies exhausted.
        let outcome = self.abort_outcome(&trigger, start, *count);
        self.record_event(swarm_id, &trigger, &outcome);
        Ok(outcome)
    }
}
```

---

## 9. Machine à états & escalade

### 9.1 Diagramme d'état

```
   ┌───────────────┐
   │     IDLE      │ (fresh engine, no collapse active)
   └───────┬───────┘
           │ handle(verdict)
           ▼
   ┌───────────────┐
   │   TRIGGER     │  trigger = from_verdict(verdict)
   └───────┬───────┘
           │
           ├── None → return NoOp
           │
           ▼
   ┌───────────────┐
   │ CIRCUIT       │  count > max → return FullAbort
   │ BREAKER       │
   └───────┬───────┘
           │
           ▼
   ┌───────────────┐
   │   ESCALATE    │◀──────────────┐
   └───────┬───────┘                │
           │                        │
       strategy.apply(ctx)          │
           │                        │
           ├─ is_recoverable → return outcome
           │                        │
           ├─ deadline reached → return DeadlineOutcome
           │                        │
           ├─ last strategy → return AbortOutcome
           │                        │
           └──── next strategy ─────┘
```

### 9.2 Exemple : NaN anomaly

```
round 3 : M5 produces NaN in latent
         ↓
round 3 : M6 → Anomaly { code: NanInf }
         ↓
M1 calls engine.handle(...)
         ↓
trigger = FromAnomaly(NanInf, round=3)
         ↓
L1 SoftRollback.applicable_to() → true
   apply: restore(round=2) → ok, stats.all_finite() → true
   → return SoftRollback outcome, success=true
         ↓
M1 continues fusion at round 3 (post-rollback)
```

### 9.3 Exemple : Variance growing

```
rounds 0-5 : M5 variance 0.01 → 0.1 → 0.5 → 1.0 → 2.0 → 5.0 (growing)
         ↓
round 5 : M6 → Diverged { reason: VarianceGrowing { slope: 1.0, window: 6 } }
         ↓
M1 calls engine.handle(...)
         ↓
trigger = FromDivergence(VarianceGrowing, round=5)
         ↓
L1 SoftRollback.applicable_to() → true, but all snapshots growing
   apply: tries round=4, 3, 2, … all have high variance → all fail
   → return with ReplayAllCorrupt warning, success=false
         ↓
L2 NoiseInjection.applicable_to() → false (not Stalled/ConfidenceDecay)
   → skip
         ↓
L3 CentroidProjection.applicable_to() → true
   apply: variance → ~0.0001, success=true
   → return CentroidProjection outcome
         ↓
M1 continues fusion at round 5 with projected latent
```

### 9.4 Circuit breaker

```
Scenarios:
- 1st collapse : count=1, proceed normally
- 2nd collapse : count=2, proceed
- …
- 5th collapse : count=5, proceed (max_collapses=5 means "allow 5")
- 6th collapse : count=6 > 5 → immediate FullAbort
```

Le compteur se reset avec `engine.reset_swarm_counter(swarm_id)` —
typiquement appelé par M1 après un respawn via M3.

---

## 10. Performance et budget

### 10.1 Budget MVP par stratégie

Pour `N=128, D=2048` :

| Strategy | Cost (ops) | Time (ms) |
|---|---|---|
| SoftRollback | N·D·2 bytes copy | ≤ 1 |
| NoiseInjection | O(N·D) + compute_stats | ~5 |
| CentroidProjection | O(N·D) + 2× compute_stats | ~7 |
| PartialRespawn | O(N·D) for scan + compute_stats | ~4 |
| FullAbort | O(1) | < 0.1 |
| **Total worst case** | escalation of all 5 | ~20 ms |

Conforme à `deadline_ms = 50`.

### 10.2 Mémoire

- Engine : `HashMap<SwarmId, u32>` + `VecDeque<CollapseEvent>` (bounded).
- 64 events × ~200 bytes = **~13 KB** par engine.
- `HashMap` : ~30 bytes per entry × expected ~100 swarms = **~3 KB**.
- Per-strategy scratch : négligeable (≤ 1 KB).

**Total : ~20 KB par engine**, partagé entre tous les swarms d'une instance.

### 10.3 Concurrence

- `engine.handle()` prend `&mut self` (history + counters) → un seul
  collapse à la fois par engine.
- Plusieurs swarms concurrents partagent une instance via `Mutex<Engine>`.
- Ou bien une instance par swarm : OK si la mémoire n'est pas contrainte.

---

## 11. Interface publique

### 11.1 Façade `efe_core::collapse`

```rust
pub use collapse::config::CollapseConfig;
pub use collapse::context::CollapseContext;
pub use collapse::engine::CollapseEngine;
pub use collapse::event::CollapseEvent;
pub use collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
pub use collapse::policy::CollapsePolicy;
pub use collapse::strategy::CollapseStrategy;
pub use collapse::trigger::CollapseTrigger;
pub use collapse::strategies::{
    CentroidProjectionStrategy, FullAbortStrategy, NoiseInjectionStrategy,
    PartialRespawnStrategy, SoftRollbackStrategy, StubStrategy,
};
```

### 11.2 Exemple — controller

```rust
let policy = CollapsePolicy::default_mvp();
let engine = Mutex::new(CollapseEngine::new(policy, CollapseConfig::default())?);

loop {
    let outcome = fusion.run_round(&mut els, &arena, &cfg, round).await?;
    let verdict = detector.ingest(&outcome.stats)?;
    match verdict {
        ConsensusVerdict::Continue => continue,
        ConsensusVerdict::Converged { .. } => break,
        _ => {
            // Collapse
            let collapse_out = engine
                .lock()
                .unwrap()
                .handle(&verdict, &mut els, &arena, fusion.clone(), round)
                .await?;
            if collapse_out.is_terminal() {
                // Respawn via M3 or abort
                break;
            }
            // Continue fusing with the recovered ELS
        }
    }
}
```

### 11.3 Custom policy

```rust
let strategies: Vec<Arc<dyn CollapseStrategy>> = vec![
    Arc::new(SoftRollbackStrategy::default()),
    Arc::new(CentroidProjectionStrategy::default()),
    Arc::new(FullAbortStrategy::default()),
];
let policy = CollapsePolicy::custom(strategies);
```

---

## 12. Stratégie de test

### 12.1 Plan de couverture

| Niveau | Exemples |
|---|---|
| Unit — chaque strategy | mock ELS, trigger matrix |
| Unit — trigger mapping | `from_verdict()` exhaustivité |
| Unit — engine orchestration | escalation order, circuit breaker |
| Integration — M5→M6→M7 | real fusion, anomaly injection, recover |
| E2E — full campaign with failure | 16 rounds, inject NaN mid-run, observe recovery |
| Property — all outcomes finite | after any strategy success, stats.all_finite() |

### 12.2 Tests unitaires

- `soft_rollback_uses_latest_snapshot`
- `soft_rollback_skips_corrupt_snapshots`
- `soft_rollback_empty_replay_returns_failure`
- `soft_rollback_rejects_dim_mismatch_trigger`
- `noise_injection_increases_variance_bounded`
- `noise_injection_deterministic_with_seed`
- `noise_injection_preserves_centroid_mean`
- `centroid_projection_variance_near_zero`
- `centroid_projection_handles_nan_centroid`
- `partial_respawn_keeps_min_active`
- `partial_respawn_rejects_below_minimum`
- `full_abort_always_applicable`
- `trigger_from_converged_is_none`
- `trigger_from_continue_is_none`
- `engine_escalates_until_success`
- `engine_returns_abort_on_exhaustion`
- `engine_circuit_breaker_fires_at_max`
- `engine_resets_counter_on_demand`
- `engine_emits_events`
- `config_validates_invariants`

### 12.3 Tests property-based

```rust
proptest! {
    #[test]
    fn noise_injection_stays_finite(
        sigma in 0.0..1.0f32,
    ) {
        // Build ELS, inject noise, check all finite.
    }

    #[test]
    fn centroid_projection_variance_small(
        initial_variance in 0.001..100.0f32,
    ) {
        // After projection, variance should be ≈ jitter² · D.
    }
}
```

### 12.4 Tests E2E

```rust
#[tokio::test]
async fn full_pipeline_recovers_from_nan_injection() {
    let (batch, mut els) = spawn_swarm(16, 16).await;
    let fusion = default_fusion();
    let mut detector = MultiCriteriaDetector::new(ConsensusConfig::default()).unwrap();
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(),
        CollapseConfig::default()
    ).unwrap();

    let arena = batch.arena_handle().unwrap().arena();
    let fusion_arc: Arc<dyn ReplayableFusion> = Arc::new(fusion);

    for r in 0..16 {
        // Inject NaN at round 5.
        if r == 5 {
            let mut split = els.split_mut(arena).unwrap();
            split.latent.row_mut(0).unwrap()[0] = bf16::from_f32(f32::NAN);
        }

        let out = fusion_arc.run_round(&mut els, arena, &cfg, r).await;
        let stats = match out {
            Ok(o) => o.stats,
            Err(_) => {
                // NaN in input caused fusion to error; trigger collapse directly
                let collapse_out = engine.handle(
                    &ConsensusVerdict::Anomaly { code: AnomalyCode::NanInf },
                    &mut els, arena, fusion_arc.clone(), r,
                ).await.unwrap();
                assert!(collapse_out.is_recoverable(), "expected recovery");
                continue;
            }
        };

        let verdict = detector.ingest(&stats).unwrap();
        if verdict.is_terminal() && !matches!(verdict, ConsensusVerdict::Converged { .. }) {
            engine.handle(&verdict, &mut els, arena, fusion_arc.clone(), r).await.unwrap();
        }
    }
    // Final ELS should still be finite.
    let stats = compute_stats(&els, arena).unwrap();
    assert!(stats.all_finite());
}
```

### 12.5 Test circuit breaker

```rust
#[tokio::test]
async fn engine_circuit_breaker_stops_loop() {
    let mut cfg = CollapseConfig::default();
    cfg.max_collapses_per_swarm = 3;
    let mut engine = CollapseEngine::new(
        CollapsePolicy::default_mvp(), cfg,
    ).unwrap();
    // Trigger 4 collapses on the same swarm.
    for i in 0..4 {
        let outcome = engine.handle(...).await.unwrap();
        if i >= 3 {
            assert!(matches!(outcome.action_taken, CollapseAction::FullAbort));
        }
    }
}
```

### 12.6 Benchmarks

- `soft_rollback_128_2048` : < 1 ms.
- `noise_injection_128_2048` : < 5 ms.
- `centroid_projection_128_2048` : < 7 ms.
- Circuit breaker : < 10 µs.

---

## 13. Dépendances inter-modules

### 13.1 Crates

```toml
[dependencies]
# workspace:
serde = { workspace = true, features = ["derive"] }
thiserror.workspace = true
half = { version = "2.4" }
rand = "0.8"
rand_pcg = "0.3"
rand_distr = "0.4"   # Normal distribution for noise injection
async-trait = "0.1"
tokio = { workspace = true, features = ["rt"] }

[dev-dependencies]
proptest = "1.4"
```

### 13.2 Contrats avec autres modules

**M5 (Fusion)** :
- `ReplayableFusion` trait déjà défini ; M7 consomme `restore_snapshot`.
- Nouveau : ajouter `fn available_replay_rounds() -> Vec<u32>` au trait.

**M4 (ELS)** :
- `compute_stats` pour post-conditions.
- `split_mut` pour perturber le latent.
- `mask_view_mut` pour désactiver des agents.

**M6 (Consensus)** :
- `ConsensusVerdict` en input (lecture seule).
- `DivergenceReason` et `AnomalyCode` dans les variants de trigger.

**M1 (Controller)** :
- Expose `CollapseEngine` via DI.
- Boucle : fuse → check → (collapse si non-convergé).

### 13.3 Extension requise à `ReplayableFusion`

```rust
#[async_trait]
pub trait ReplayableFusion: FusionStrategy {
    async fn record_snapshot(&self, round: u32, els: &EpistemicLatentSpace, arena: &Arena) -> Result<()>;
    async fn restore_snapshot(&self, round: u32, els: &mut EpistemicLatentSpace, arena: &Arena) -> Result<()>;

    /// **New for M7**: enumerate rounds currently in the replay buffer,
    /// newest-first. Returns empty if replay is disabled.
    fn available_replay_rounds(&self) -> Vec<u32>;
}
```

L'implémentation sur `MvpCpuFusion` délègue à `ReplayBuffer::rounds()` en
inverse.

---

## 14. Plan de sprint M7

### 14.1 Sprint 6 — MVP (1 semaine)

**J1-J2** :
- Scaffolding `collapse/`.
- `CollapseConfig`, `CollapseTrigger`, `CollapseAction`, `CollapseOutcome`, `CollapseWarning`.
- Tests unit des types.

**J3** :
- `CollapseStrategy` trait + `StubStrategy` (tests).
- `SoftRollbackStrategy` + tests.

**J4** :
- `NoiseInjectionStrategy` + tests.
- `CentroidProjectionStrategy` + tests.

**J5** :
- `PartialRespawnStrategy` + tests.
- `FullAbortStrategy` + tests.

**J6** :
- `CollapsePolicy` + `CollapseEngine`.
- Escalation loop, circuit breaker.
- Tests orchestration.

**J7** :
- Extension `ReplayableFusion::available_replay_rounds`.
- Tests E2E M5→M6→M7.
- Benchmarks.
- Doc complète.

### 14.2 Sprint 7 — Tuning (0.5 semaine)

- Scratch pool dans engine (zéro alloc hot-path).
- `estimate_recovery_confidence` sophistiqué (via history).
- Adaptive escalation (apprendre quelle stratégie marche pour quel trigger).
- Ingest d'events pour M11.

### 14.3 Sprint 8+ — Meta-learning

- Policy apprise sur runs historiques.
- Collapse prédictif (anticiper divergence avant M6).

---

## 15. Critères d'acceptation

### 15.1 Fonctionnels

- [ ] Injection de NaN dans le latent → collapse réussit (rollback).
- [ ] Variance explosion → collapse réussit (rollback ou projection).
- [ ] Stall → noise injection sort du plateau.
- [ ] Circuit breaker arrête la boucle après `max_collapses_per_swarm`.
- [ ] Après reset_swarm_counter, un nouveau collapse est permis.
- [ ] Post-collapse, `compute_stats(els).all_finite() == true`.
- [ ] Events sont émis et consommables par M11.

### 15.2 Qualité

- [ ] `cargo fmt --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean pedantic.
- [ ] `cargo doc --no-deps --workspace` clean.
- [ ] Couverture ≥ 90% sur `collapse/`.
- [ ] Zéro `unsafe` dans M7 (délègue à M4 via split_mut).

### 15.3 Performance

- [ ] `SoftRollback` < 2 ms pour N=128, D=2048.
- [ ] `NoiseInjection` < 10 ms.
- [ ] `CentroidProjection` < 10 ms.
- [ ] Full escalation worst-case < 50 ms.
- [ ] Memory footprint par engine < 50 KB.

### 15.4 Observabilité

- [ ] Chaque outcome contient `duration_us`, `strategy_name`.
- [ ] `CollapseEvent` sérialisable en JSON pour M11.
- [ ] Tracing span `collapse.handle`, `collapse.{strategy_name}`.

---

## 16. Risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Collapse s'exécute pendant la fusion (race) | Bas | Critique | `&mut EpistemicLatentSpace` enforcé par Rust. Test concurrent impossible (compile-time). |
| R2 | `SoftRollback` sur snapshot corrompu | Moyen | Élevé | Post-restore `compute_stats().all_finite()` check, retry older. |
| R3 | `NoiseInjection` augmente variance au-delà du threshold | Moyen | Moyen | Post-variance check, warning `NoiseIneffective`, escalade L3. |
| R4 | `PartialRespawn` laisse moins de `min_active` agents | Moyen | Élevé | Pré-check, fail avec `PartialRespawnBelowMinimum`. |
| R5 | Circuit breaker trop agressif | Moyen | Moyen | Configurable; defaults généreux (5 collapses). |
| R6 | Boucle infinie collapse → fusion → collapse | Moyen | Critique | Circuit breaker obligatoire, `FullAbort` garantit terminaison. |
| R7 | PRNG non-déterministe en CI | Bas | Bas | `deterministic_seed: true` dans CI config. |
| R8 | `CollapseEvent` buffer déborde | Bas | Bas | Ring buffer bounded. |
| R9 | `available_replay_rounds` buggy | Bas | Élevé | Couvert par test dédié avec mock. |
| R10 | Deadline non respectée (stratégie hangs) | Bas | Moyen | Check deadline entre stratégies ; stratégie individuelle idempotente. |

---

## 17. Annexes

### 17.1 Glossaire

| Terme | Définition |
|---|---|
| **Collapse** | Action de rendre le swarm à un état défini après échec. |
| **Trigger** | Cause du collapse : verdict M6 ou demande manuelle. |
| **Strategy** | Une des 5 actions : rollback, noise, projection, respawn, abort. |
| **Policy** | Liste ordonnée de stratégies à essayer en escalade. |
| **Circuit breaker** | Limite au nombre de collapses par swarm. |
| **Post-condition** | Invariant vérifié après collapse (finite stats). |
| **Recovery confidence** | Estimation [0,1] que la fusion convergera après collapse. |
| **Event** | Enregistrement télémétrique d'un collapse. |

### 17.2 Fonction utilitaire : `estimate_recovery_confidence`

```rust
fn estimate_recovery_confidence(stats: &ElsStats, cfg: &CollapseConfig) -> f32 {
    if !stats.all_finite() { return 0.0; }
    let variance_factor = 1.0 - (stats.variance / cfg.post_variance_max).clamp(0.0, 1.0);
    let outlier_factor = 1.0 - (stats.outlier_ratio * 5.0).clamp(0.0, 1.0);
    let active_factor = if stats.n_active >= 16 { 1.0 } else { stats.n_active as f32 / 16.0 };
    (variance_factor * 0.5 + outlier_factor * 0.3 + active_factor * 0.2).clamp(0.0, 1.0)
}
```

Heuristique simple pour MVP; Sprint 7+ la remplace par un modèle ML.

### 17.3 `default_mvp` policy

```rust
impl CollapsePolicy {
    pub fn default_mvp() -> Self {
        Self {
            escalation_order: vec![
                Arc::new(SoftRollbackStrategy::default()),
                Arc::new(NoiseInjectionStrategy::default()),
                Arc::new(CentroidProjectionStrategy::default()),
                Arc::new(PartialRespawnStrategy::default()),
                Arc::new(FullAbortStrategy::default()),
            ],
            max_retries_per_call: 1,
        }
    }
}
```

### 17.4 Exemple numérique : rollback après NaN

**Setup** : `N=4, D=2`, 3 rounds de fusion, round 3 injecte NaN.

| Round | variance | has_nan | confidence | snapshot stored? |
|---|---|---|---|---|
| 0 | 0.50 | false | 0.30 | yes |
| 1 | 0.25 | false | 0.45 | yes |
| 2 | 0.10 | false | 0.65 | yes |
| 3 | NaN | true | — | no (fast-path) |

M6 : `Anomaly { code: NanInf }` au round 3.
M1 appelle `engine.handle(verdict, els, arena, fusion, 3)`.
Engine : `L1 SoftRollback` :
- restore round=2 → ok, stats.variance=0.10, all_finite=true.
- return SoftRollback { to_round: 2 }, success=true.

Controller continue fusion at round 3 avec latent restauré.

### 17.5 Diagramme : escalade pour `VarianceGrowing`

```
trigger = VarianceGrowing(slope=1.0, window=6)

L1 SoftRollback applicable? YES
  apply: all snapshots have high variance (growing trend)
  → warning: ReplayAllCorrupt, success=false

L2 NoiseInjection applicable? NO (trigger is not Stall/ConfidenceDecay)
  → skip

L3 CentroidProjection applicable? YES
  apply: variance = ~(jitter^2 * D) = ~0.0001
  → success=true, action=CentroidProjection
  → return outcome
```

### 17.6 Checklist d'audit post-implémentation

- [ ] Chaque stratégie a au moins 3 tests : applicability, success case, failure mode.
- [ ] `engine.handle` sur un `Continue` verdict retourne `NoOp`.
- [ ] `engine.handle` sur un `Converged` verdict retourne `NoOp`.
- [ ] Deadline respectée : cumul des durées des stratégies ≤ `deadline_ms`.
- [ ] `CollapseEvent::timestamp_nanos` est monotone.
- [ ] Les warnings sont stables (pas de rename entre sprints).
- [ ] `is_recoverable()` et `is_terminal()` sont exhaustifs (tous les cas).
- [ ] Post-condition vérifiée via `validate_post_collapse(els, arena)`.

### 17.7 Exemple d'event JSON (pour M11)

```json
{
  "timestamp_nanos": 1699876543210000000,
  "swarm_id": "550e8400-e29b-41d4-a716-446655440000",
  "trigger": {
    "FromDivergence": {
      "reason": { "VarianceExplosion": { "variance": 150.7 } },
      "round": 5
    }
  },
  "outcome": {
    "action_taken": { "SoftRollback": { "to_round": 3 } },
    "success": true,
    "new_round": 3,
    "variance_after": 0.08,
    "recovery_confidence": 0.87,
    "warnings": [],
    "duration_us": 820,
    "strategy_name": "soft-rollback"
  },
  "collapse_index": 1
}
```

---

**Fin du document M7 ARCHITECTURE.md** — prêt pour implémentation sprint 6.
