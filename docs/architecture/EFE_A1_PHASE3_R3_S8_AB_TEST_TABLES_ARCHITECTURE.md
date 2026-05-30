# EFE A1 — Phase 3 R3.S8 — A/B test entre tables de scoring

> **Statut** : architecture (à implémenter)
> **Sprint** : Phase 3 R3.S8
> **Priorité roadmap** : 🟡 medium
> **Pré-requis** : R3.S5 (résolution de table per-tenant), R3.S6 (déterminisme
> seedé), R3.S7 (attribution métriques), S6-R2 (`TaskClassScoringTable`)
> **Fichiers cible** : `crates/efe-coordinator/src/ab_test.rs` (NEW),
> `coordinator.rs`, `workflows/mission_a.rs`, `metrics.rs`, `events/stream.rs`,
> `savings_report.rs`, `lib.rs`, `tests/ab_test.rs` (NEW)

---

## 0. Résumé exécutif

La `TaskClassScoringTable` (calibration S6, figée nov-2025) décide vers quel
LLM router chaque `task_class`. Quand le paysage change (GPT-5 sort, prix
révisés), on veut **tester une nouvelle table en prod sans risque** : router un
pourcentage du trafic vers `table_v2` et comparer empiriquement
coût/qualité/latence des deux bras.

R3.S8 livre un **framework d'A/B test de tables** :

1. **`ABTestConfig`** — `control_table` (v1) vs `experiment_table` (v2),
   `experiment_traffic_ratio`, `tenant_filter`, `fixture_filter`.
2. **Bucketing déterministe** `blake3(swarm_id) % 100` (zéro dépendance `rand`,
   reproductible : même `swarm_id` → même bras).
3. **Attribution** — l'arme (`ABArm::Control|Experiment`) est tracée dans
   `ChewingMetrics.ab_arm`, propagée au `RunRecord`, et le `SavingsReport`
   **ségrège** les runs par bras pour comparaison.
4. **Test live** `live_ab_test_savings_report` — N missions, 50/50, rapport
   segmenté control vs experiment.

**Principes directeurs (hérités R3.S3→S7)** :
- **Zéro dépendance** : bucketing via `blake3` (déjà dépendance audit), pas de
  `rand`.
- **Zéro dead config** : chaque champ d'`ABTestConfig` a un point d'effet réel ;
  `fixture_filter` n'a de sens que dans le harness de report (pas d'identité
  fixture en prod) → documenté honnêtement (§4.4), pas exposé comme contrôle
  mort en live.
- **Précédence claire & sûre** : `request table > A/B > tenant > global`
  (§4.1) ; A/B **Mission A uniquement** (seule mission avec table/router).
- **Backward-compat** : `ab_test: None` ⇒ comportement R3.S7 inchangé.
- **Déterminisme** : critère d'acceptation central — même `swarm_id` → même
  bucket (reproductibilité statistique).

---

## 1. Scope / Non-scope

### 1.1 Dans le scope

| # | Livrable | Détail |
|---|---|---|
| 1 | `ABArm` (Control/Experiment) + `ABTestConfig` (sérialisable) | `ab_test.rs` |
| 2 | `ABTestRuntime` (Arc tables, zéro clone/mission) + `assign_arm` + `bucket_of` | `ab_test.rs` |
| 3 | Bucketing déterministe `blake3(swarm_id)` | `bucket_of` |
| 4 | `Coordinator.ab_test: Option<Arc<ABTestRuntime>>` + builder | `coordinator.rs` |
| 5 | Insertion dans la résolution de table Mission A (précédence §4.1) | `mission_a.rs` |
| 6 | `ChewingMetrics.ab_arm: Option<ABArm>` | `metrics.rs` |
| 7 | `SavingsReport` ségrégation par bras (`ab_comparison`) | `savings_report.rs` |
| 8 | 7 tests offline + 2 tests live | `tests/ab_test.rs` |

### 1.2 Hors scope (déféré / honnête, §15)

- **Significativité statistique online** (p-value, intervalles de confiance,
  arrêt séquentiel) — S8 livre la **ségrégation** (stats descriptives par bras)
  ; le test d'hypothèse formel est un sprint d'analyse à part (§4.6).
- **A/B Mission B / Speculative** — ces missions n'utilisent **pas** de table
  de scoring (B = fan-out sans router ; Spec = drafter/verifier fixés). A/B est
  **Mission-A-only** (§4.7).
- **Sticky bucketing par utilisateur** — le bucket est par `swarm_id` (par
  mission), choix de la roadmap, adapté à un A/B de *routing* (§4.2).
- **Multi-variant (A/B/C/n)** — S8 = 2 bras (control/experiment). Multi-bras =
  extension future.
- **Label `ab_arm` sur les métriques Prometheus (R3.S7)** — optionnel ; le
  mécanisme d'attribution primaire est le `SavingsReport` (offline). Exposer un
  label `ab_arm` est borné (2-3 valeurs) donc sûr en cardinalité, mais hors
  scope du livrable principal (§15).

### 1.3 Invariant de non-régression

> `ab_test: None` (défaut) ⇒ aucune assignation, `req.task_class_scoring_table`
> résolu exactement comme R3.S5/S7 ⇒ comportement byte-pour-byte. `ABArm`
> ajouté à `ChewingMetrics` est `#[serde(default)]` (= `None`) ⇒ rétrocompat
> JSON.

---

## 2. Problème

La table de scoring est un **singleton calibré une fois** (`default_3llm_table`,
`scoring.rs:236`, version `"s6-r2-…"`). Toute évolution (nouveau LLM, prix
révisés, capacités re-jugées) exige une nouvelle table — mais la déployer
d'un coup en prod est **risqué** : si la calibration v2 est mauvaise, *tout* le
trafic route mal (coût↑ ou qualité↓) sans filet.

Il manque un moyen de **router un % du trafic vers v2** et de **mesurer** si v2
est meilleure (coût/qualité/latence) avant bascule globale. C'est un A/B test
classique, appliqué aux tables de scoring.

---

## 3. État actuel du code (inventaire ancré)

| Élément | Emplacement | Constat |
|---|---|---|
| `TaskClassScoringTable` (capability, static_metadata, weights_per_task, `version`) ; `Clone+Default+Serde` | `efe-nano-app-coordinator/.../scoring.rs:207` | la table A/B-able ; `version` = label de bras |
| Override table per-request | `mission_a.rs` `req.task_class_scoring_table` → `RouterClient::for_task_class` (L195) | point de branchement |
| Résolution R3.S5 (request > tenant > global) | `mission_a.rs:89-116` | A/B s'insère entre request et tenant |
| `swarm_id = Uuid::now_v7()` | `mission_a.rs:57` | clé de bucketing |
| `ChewingMetrics` (tenant_id, tenant_cost_cap_exceeded, tenant_llms_filtered, …) | `metrics.rs:43-…` | ajouter `ab_arm` à la fin |
| Littéraux `ChewingMetrics` | `metrics.rs` (test serde), `events/stream.rs` (test I3) | **2 sites à patcher** (tripwire R3.S5/S7) |
| `RunMode` (Baseline/S4Chewing/S6Routing/Full/Speculative) | `savings_report.rs:49-67` | A/B est **orthogonal** au mode |
| `RunRecord { …, chewing_metrics: Option<ChewingMetrics>, passed_quality, … }` | `savings_report.rs:117-149` | porte déjà `chewing_metrics` → arm accessible |
| `RunRecord::new(...)` (11 args) | `savings_report.rs:170` | **NE PAS** changer la signature (lit l'arm via chewing_metrics) |
| `ModeSummary { quality_pass_rate, avg_cost_microcents, … }` | `savings_report.rs:253-281` | modèle pour `ArmStats` |
| `Coordinator{…, policies, metrics}` + builder | `coordinator.rs:180-…` | ajouter `ab_test` (même pattern) |
| `blake3 = "1.5"` (dépendance directe) | `Cargo.toml:43` | hash bucketing **zéro nouvelle dep** |
| Pas de `rand` | `Cargo.toml` | confirme blake3 |
| R3.S7 `MissionType`/metrics | `metering.rs` | label `ab_arm` possible (hors scope §15) |

**Observation décisive** : `RunRecord` porte déjà `chewing_metrics` → si
l'arm vit dans `ChewingMetrics.ab_arm`, la ségrégation du report se fait sans
changer la signature `RunRecord::new` (11 args, nombreux call sites). C'est le
choix minimal-churn (§4.5).

---

## 4. Décisions chirurgicales

### 4.1 Précédence de résolution de table (Mission A)

A/B s'insère dans la chaîne R3.S5. Ordre (du plus prioritaire au moins) :

| Priorité | Source | Règle |
|---|---|---|
| 1 | `req.task_class_scoring_table` (pin explicite) | le caller a choisi → **pas d'A/B** |
| 2 | **A/B** (config active ∧ tenant/fixture dans la population) | bucket(swarm_id) → control \| experiment |
| 3 | `policy.scoring_table` (tenant, R3.S5) | si A/B inactif |
| 4 | router global | fallback |

Rationale : (1) un caller qui épingle une table veut *cette* table, on ne
l'expérimente pas. (2) l'A/B est une décision opérée délibérément → il prime
sur le défaut tenant pour la population ciblée (c'est *le but* : tester une
table candidate à la place de la table courante). (3-4) inchangé R3.S5.

### 4.2 Bucketing déterministe `blake3(swarm_id)` (zéro `rand`)

```text
bucket = u64::from_le_bytes(blake3(swarm_id.as_bytes())[0..8]) % 100   ∈ [0, 99]
arm    = if (bucket as f32) < experiment_traffic_ratio * 100.0 { Experiment } else { Control }
```

- **Déterministe** : `blake3` est pur → même `swarm_id` → même bucket → même
  bras (critère d'acceptation central). Testable sans réseau.
- **Uniforme** : `blake3` distribue uniformément (contrairement à
  `uuid_v7 % 100` qui biaiserait — les bits de poids fort de v7 sont
  temporels). On hashe pour casser cette structure.
- **Zéro dépendance** : `blake3` est déjà tiré (audit chain).
- **Par mission** (`swarm_id` frais à chaque mission) : adapté à un A/B de
  *routing* (la table n'affecte pas la continuité d'UX). Pour du sticky-user,
  on hasherait un id stable — hors scope (§1.2), choix roadmap.
- **`ratio = 0`** ⇒ seuil `0.0` ⇒ `bucket < 0` jamais ⇒ **toujours Control**
  (l'expérience est désactivée, test `…_traffic_ratio_zero_skips_experiment`).
  `ratio = 1` ⇒ seuil `100.0` ⇒ toujours Experiment.

### 4.3 `ABTestConfig` (sérialisable) vs `ABTestRuntime` (Arc, zéro clone/mission)

La roadmap montre `ABTestConfig { control_table: TaskClassScoringTable, … }`
(tables **owned**) — nécessaire pour `Serialize` (le test
`ab_test_config_serializes_stable`) ; or `serde` d'un `Arc<T>` exige la feature
`rc` (non activée). **Mais** router via `req.task_class_scoring_table:
Option<Arc<TaskClassScoringTable>>` exigerait un `clone()` de la table (des
HashMaps) **à chaque mission A/B** — gâchis.

**Décision** : deux types.
- **`ABTestConfig`** (entrée user, **owned** tables) — `Serialize/Deserialize`,
  c'est ce que l'opérateur écrit/charge.
- **`ABTestRuntime`** — construit **une fois** par `builder.ab_test(config)` :
  pré-emballe chaque table dans un `Arc<TaskClassScoringTable>`. La résolution
  par mission = un `Arc::clone` (compteur atomique), **zéro clone profond**.

Le `Coordinator` détient `Option<Arc<ABTestRuntime>>`. Ainsi : config
sérialisable + résolution O(1) par mission.

### 4.4 `tenant_filter` (population) + `fixture_filter` (report-only, honnête)

- **`tenant_filter: Option<TenantId>`** : `None` ⇒ tous les tenants dans la
  population A/B ; `Some(t)` ⇒ seul `t` est expérimenté, les autres tenants
  retombent sur la résolution normale (R3.S5), **pas** sur `control_table`
  (ils ne font pas partie de l'expérience). `assign_arm` renvoie `None` pour un
  tenant hors filtre → la chaîne R3.S5 reprend (test `ab_test_with_tenant_filter`).
- **`fixture_filter: Option<Vec<String>>`** : un "fixture name" n'existe **que**
  dans le harness de report/test (une `MissionARequest` prod n'a pas d'identité
  fixture). **Décision honnête** : `assign_arm` prend un `fixture: Option<&str>`
  ; le **workflow live passe `None`** → `fixture_filter` ne contraint **que**
  quand un nom est fourni (harness). Quand `fixture_filter = Some(list)` et
  `fixture = None` (live) → le mission n'est **pas** dans la population (on ne
  peut pas confirmer l'appartenance) → `None` (résolution normale). Ainsi
  `fixture_filter` n'est pas du dead-config : il a un effet **réel dans le
  harness** (où le nom est connu), et un comportement **défini & sûr** en live
  (exclusion). Documenté, pas masqué (cf. `EFE_CHAOS_RATE` R3.S6).

### 4.5 Attribution via `ChewingMetrics.ab_arm` (minimal-churn)

`RunRecord` porte déjà `chewing_metrics` (§3). On ajoute
`ChewingMetrics.ab_arm: Option<ABArm>` (`#[serde(default)]`), posé par le
workflow au moment de l'assignation. La ségrégation du `SavingsReport` lit
`record.chewing_metrics.and_then(|cm| cm.ab_arm)` → **aucun changement de la
signature `RunRecord::new`** (11 args, nombreux call sites). Helper
`RunRecord::ab_arm()`.

`ABArm` est défini dans `ab_test.rs` ; `metrics.rs` l'importe (`use
crate::ab_test::ABArm`). Dépendance **acyclique** (`ab_test` n'importe pas
`metrics`).

### 4.6 Statistiques : ségrégation, pas de p-value online

Le test d'hypothèse formel (Welch t-test, intervalles, arrêt séquentiel) est un
domaine à part. S8 livre la **ségrégation descriptive** : par bras,
`n_runs`, `avg_cost_microcents`, `quality_pass_rate`, `avg_latency_ms`, +
**delta** experiment-vs-control. L'opérateur (ou un futur sprint d'analyse)
juge la significativité. C'est l'honnêteté de périmètre (cf. R3.S5
`retry_policy` descopé, R3.S7 `quality_pass_rate` reframé).

### 4.7 A/B Mission A uniquement

Seule Mission A a une `task_class_scoring_table` → un router → un comportement
modulable par table. Mission B (fan-out, pas de router) et Speculative
(drafter/verifier fixés, `req.task_class_scoring_table` ignoré) **n'utilisent
pas** la table. A/B est donc **Mission-A-scoped** (comme le scoring R3.S5). Le
workflow B/Spec n'appelle pas l'A/B. Documenté.

---

## 5. Modèle de données (`src/ab_test.rs`)

```rust
// SPDX-License-Identifier: UNLICENSED
//! R3.S8 — A/B test entre tables de scoring (bucketing déterministe).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use efe_nano_app_coordinator::types::TaskClassScoringTable;
use efe_nano_app_coordinator::TenantId;

/// Bras d'un A/B test. `#[serde(rename_all="SCREAMING_SNAKE_CASE")]` cohérent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ABArm {
    /// Table de contrôle (v1, prod actuelle).
    Control,
    /// Table d'expérimentation (v2, candidate).
    Experiment,
}
impl ABArm {
    #[must_use] pub fn as_str(self) -> &'static str {
        match self { Self::Control => "control", Self::Experiment => "experiment" }
    }
}

/// Configuration A/B **sérialisable** (entrée user). Tables owned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTestConfig {
    /// Table de contrôle (v1).
    pub control_table: TaskClassScoringTable,
    /// Table d'expérimentation (v2).
    pub experiment_table: TaskClassScoringTable,
    /// Fraction du trafic routée vers l'expérience ∈ [0,1].
    pub experiment_traffic_ratio: f32,
    /// Si `Some`, seul ce tenant est dans la population A/B.
    #[serde(default)]
    pub tenant_filter: Option<TenantId>,
    /// Si `Some`, seules ces fixtures (harness) sont dans la population.
    #[serde(default)]
    pub fixture_filter: Option<Vec<String>>,
}

/// État runtime : tables pré-emballées en `Arc` (zéro clone profond/mission).
#[derive(Debug, Clone)]
pub struct ABTestRuntime {
    control: Arc<TaskClassScoringTable>,
    experiment: Arc<TaskClassScoringTable>,
    ratio: f32,
    tenant_filter: Option<TenantId>,
    fixture_filter: Option<Vec<String>>,
}

impl ABTestRuntime {
    /// Construit le runtime depuis une config (Arc-wrap une fois).
    #[must_use]
    pub fn from_config(cfg: ABTestConfig) -> Self {
        Self {
            control: Arc::new(cfg.control_table),
            experiment: Arc::new(cfg.experiment_table),
            ratio: cfg.experiment_traffic_ratio.clamp(0.0, 1.0),
            tenant_filter: cfg.tenant_filter,
            fixture_filter: cfg.fixture_filter,
        }
    }

    /// Bucket déterministe ∈ [0,99] depuis `swarm_id` (blake3, §4.2).
    #[must_use]
    pub fn bucket_of(swarm_id: uuid::Uuid) -> u64 {
        let h = blake3::hash(swarm_id.as_bytes());
        let b = h.as_bytes();
        u64::from_le_bytes([b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7]]) % 100
    }

    /// Assigne un bras, ou `None` si la mission n'est pas dans la population
    /// (tenant/fixture hors filtre) → la résolution normale (R3.S5) reprend.
    #[must_use]
    pub fn assign_arm(&self, swarm_id: uuid::Uuid, tenant: TenantId, fixture: Option<&str>) -> Option<ABArm> {
        if let Some(ft) = self.tenant_filter {
            if ft != tenant { return None; }
        }
        if let Some(ff) = &self.fixture_filter {
            match fixture {
                Some(name) if ff.iter().any(|f| f == name) => {}
                _ => return None, // pas de nom (live) ou hors liste → exclu
            }
        }
        let bucket = Self::bucket_of(swarm_id);
        Some(if (bucket as f32) < self.ratio * 100.0 { ABArm::Experiment } else { ABArm::Control })
    }

    /// Table `Arc` du bras (clone du compteur atomique, pas de clone profond).
    #[must_use]
    pub fn table_for(&self, arm: ABArm) -> Arc<TaskClassScoringTable> {
        match arm { ABArm::Control => self.control.clone(), ABArm::Experiment => self.experiment.clone() }
    }
}
```

---

## 6. Intégration

### 6.1 Coordinator + builder

```rust
// coordinator.rs
pub ab_test: Option<Arc<ABTestRuntime>>,    // None ⇒ A/B off (défaut)

// builder
fn ab_test: Option<Arc<ABTestRuntime>>
#[must_use] pub fn ab_test(mut self, cfg: ABTestConfig) -> Self {
    self.ab_test = Some(Arc::new(ABTestRuntime::from_config(cfg)));
    self
}
// build(): ab_test: self.ab_test ; for_tests(): None
```

### 6.2 Insertion dans `run_mission_a` (résolution de table)

Entre la résolution tenant (L89) et l'injection de la table tenant (L109-111),
on insère l'A/B — qui ne s'applique que si la requête n'a **pas** épinglé de
table (précédence §4.1) :

```rust
let (tenant_id, policy) = multi_tenant::resolve(...);   // L89 (existant)
// … allow-list + budget (existant) …

// R3.S8 : A/B test (précédence : request > A/B > tenant > global).
if req.task_class_scoring_table.is_none() {
    if let Some(ab) = &coord.ab_test {
        if let Some(arm) = ab.assign_arm(swarm_id, tenant_id, /*fixture*/ None) {
            req.task_class_scoring_table = Some(ab.table_for(arm));
            chewing_metrics.ab_arm = Some(arm);
        }
    }
}
// L109-111 (existant) : injecte la table tenant SEULEMENT si toujours None.
if req.task_class_scoring_table.is_none() {
    req.task_class_scoring_table = policy.and_then(|p| p.scoring_table.clone());
}
```

`fixture = None` en live (§4.4). Le router (L195) consomme
`req.task_class_scoring_table` tel quel → **aucun churn** sur le bloc router.

### 6.3 ChewingMetrics

```rust
/// **R3.S8** — Bras A/B assigné (None = pas d'A/B sur cette mission).
#[serde(default)]
pub ab_arm: Option<crate::ab_test::ABArm>,
```

Tripwire : patcher les **2 littéraux** `ChewingMetrics` (test serde
`metrics.rs`, test I3 `events/stream.rs`).

### 6.4 SavingsReport — ségrégation par bras

```rust
// savings_report.rs
impl RunRecord {
    /// Bras A/B de ce run (lu depuis chewing_metrics ; None si pas d'A/B).
    #[must_use] pub fn ab_arm(&self) -> Option<ABArm> {
        self.chewing_metrics.as_ref().and_then(|cm| cm.ab_arm)
    }
}

/// Stats descriptives d'un bras.
pub struct ArmStats {
    pub arm: ABArm,
    pub table_version: String,    // depuis la table du bras (traçabilité)
    pub n_runs: usize,
    pub avg_cost_microcents: f64,
    pub quality_pass_rate: f32,
    pub avg_latency_ms: f32,
}
/// Comparaison control vs experiment (deltas).
pub struct ABComparison {
    pub control: ArmStats,
    pub experiment: ArmStats,
    pub cost_delta_microcents: f64,   // exp - ctrl (négatif = exp moins cher)
    pub quality_delta: f32,
    pub latency_delta_ms: f32,
}
impl SavingsReport {
    /// Ségrège les runs par bras A/B → comparaison. `None` si aucun run A/B.
    #[must_use] pub fn ab_comparison(&self) -> Option<ABComparison> { /* group_by ab_arm */ }
    /// Section markdown (R3.S9 `to_markdown_v2` l'inclut).
    #[must_use] pub fn ab_section_markdown(&self) -> String { /* … */ }
}
```

`table_version` provient de `TaskClassScoringTable.version` — relie le bras à
la calibration testée (audit/report).

---

## 7. Plan de tests (7 offline + 2 live)

### Offline (`tests/ab_test.rs` + unit `ab_test.rs`)
1. `ab_test_config_serializes_stable` — `ABTestConfig` round-trip JSON stable.
2. `ab_test_assigns_buckets_deterministically` — même `swarm_id` → même bras
   sur 1000 répétitions ; deux swarm_id distincts peuvent différer.
3. `ab_test_bucket_distribution_matches_ratio` — sur 10 000 swarm_id, fréquence
   Experiment ≈ ratio (±3 pts à 0.30).
4. `ab_test_traffic_ratio_zero_skips_experiment` — ratio 0 → toujours Control.
5. `ab_test_traffic_ratio_one_always_experiment` — ratio 1 → toujours Experiment.
6. `ab_test_with_tenant_filter` — tenant hors filtre → `assign_arm` None
   (résolution R3.S5 reprend) ; tenant filtré → bucket décide.
7. `ab_test_savings_report_segregates_control_vs_experiment` — runs étiquetés
   par bras (via chewing_metrics.ab_arm) → `ab_comparison` calcule n_runs +
   coûts + qualité par bras + deltas.

### Intégration (workflow)
8. `coordinator_ab_routes_and_traces_arm` — `Coordinator` avec `ABTestConfig`
   (control/experiment tables qui routent différemment) → `run_mission_a` →
   `chewing_metrics.ab_arm == Some(arm)` cohérent avec `bucket_of(swarm_id)`,
   et `llm_used` correspond à la table du bras.
9. `request_pinned_table_skips_ab` — requête avec `task_class_scoring_table`
   épinglée → `ab_arm == None` (priorité §4.1).

### Live (gate `EFE_LIVE_LLM`)
- `live_ab_test_savings_report` — N missions (≥ 30) réparties 50/50 sur 2
  tables réelles (control = `default_3llm_table`, experiment = variante), 5
  fixtures → `SavingsReport` segmenté + rapport markdown control vs experiment.
- `live_ab_test_deterministic_replay` — rejouer les mêmes `swarm_id` → mêmes
  bras (reproductibilité en condition réelle).

---

## 8. Backward-compat & déterminisme

- **`ab_test: None`** ⇒ le bloc §6.2 est court-circuité (`coord.ab_test` None)
  ⇒ résolution R3.S5/S7 inchangée, byte-pour-byte (invariant §1.3, test #9 sans
  config).
- **`ChewingMetrics.ab_arm`** `#[serde(default)]` ⇒ JSON pré-S8 désérialise à
  `None`.
- **Déterminisme** : `bucket_of` est pur (`blake3`) ⇒ reproductible (tests #2,
  live replay). Aucune source d'entropie/horloge dans l'assignation.
- **Sécurité cardinalité** : si un label `ab_arm` est un jour ajouté aux
  métriques R3.S7, il est borné (control/experiment) — mais hors scope (§1.2).

---

## 9. Plan de modification fichier-par-fichier

| Fichier | Action | Détail |
|---|---|---|
| `src/ab_test.rs` | **NEW** | `ABArm`, `ABTestConfig`, `ABTestRuntime` (`from_config`/`bucket_of`/`assign_arm`/`table_for`) + tests unit |
| `src/lib.rs` | patch | `pub mod ab_test;` + re-exports `ABArm`, `ABTestConfig`, `ABTestRuntime` |
| `src/coordinator.rs` | patch | champ `ab_test` + builder `.ab_test()` + `build()`/`for_tests()` |
| `src/workflows/mission_a.rs` | patch | bloc A/B §6.2 (précédence) |
| `src/metrics.rs` | patch | `ChewingMetrics.ab_arm` + littéral test |
| `src/events/stream.rs` | patch | littéral `ChewingMetrics` test I3 |
| `src/savings_report.rs` | patch | `RunRecord::ab_arm()`, `ArmStats`, `ABComparison`, `ab_comparison()`, `ab_section_markdown()` |
| `tests/ab_test.rs` | **NEW** | tests #1-#9 |
| `tests/live_missions.rs` (ou dédié) | patch | 2 tests live |
| `CHANGELOG.md` | patch | section R3.S8 |

---

## 10. Phasage d'implémentation (12 phases)

1. **Phase 1** — `ab_test.rs` : `ABArm` + `ABTestConfig` + serde test (#1).
2. **Phase 2** — `ABTestRuntime::from_config` + `bucket_of` (blake3) + tests
   déterminisme/distribution (#2, #3).
3. **Phase 3** — `assign_arm` (ratio 0/1, tenant_filter, fixture_filter) +
   `table_for` + tests #4, #5, #6.
4. **Phase 4** — `lib.rs` re-exports. `cargo check`.
5. **Phase 5** — `metrics.rs` `ab_arm` + littéral ; `events/stream.rs` littéral.
6. **Phase 6** — `coordinator.rs` champ + builder + build/for_tests.
7. **Phase 7** — `mission_a.rs` bloc A/B (précédence §6.2). Compile-check.
8. **Phase 8** — `savings_report.rs` `ab_arm()` + `ArmStats`/`ABComparison`/
   `ab_comparison()` + test #7.
9. **Phase 9** — `tests/ab_test.rs` intégration (#8, #9).
10. **Phase 10** — tests live (compile + skip sans clés).
11. **Phase 11** — suite complète + `clippy --all-targets` exit 0 + vérifier
    invariant non-régression (ab_test None).
12. **Phase 12** — CHANGELOG + commit.

---

## 11. Risques & mitigations

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| Bucketing biaisé (`uuid % 100`) | moyen | élevé | hash `blake3` (uniforme), test distribution #3 |
| Clone profond table/mission | moyen | moyen | `ABTestRuntime` Arc pré-emballé (§4.3) |
| `fixture_filter` dead-config en live | moyen | bas | exclusion définie + documentée (§4.4) |
| Régression résolution table (R3.S5) | faible | élevé | invariant §1.3 + test #9 (pin) + test sans config |
| Littéraux `ChewingMetrics` oubliés | moyen | bas | tripwire connu (compile-check tests) |
| A/B appliqué à B/Spec par erreur | faible | moyen | A/B Mission-A-only, pas d'appel en B/Spec (§4.7) |
| Sérialisation `Arc` (serde rc) | faible | bas | config owned (Serialize) ≠ runtime Arc (§4.3) |
| Sur-ingénierie stats | — | bas | ségrégation descriptive seule (§4.6) |

---

## 12. Critères d'acceptation

- [ ] 7 tests offline PASS (config serde, bucketing déterministe, distribution,
      ratio 0/1, tenant_filter, ségrégation report).
- [ ] 2 tests intégration (routing + arm tracé ; pin skip A/B).
- [ ] 2 tests live PASS → rapport segmenté control vs experiment.
- [ ] **Bucketing reproductible** : même `swarm_id` → même bras (test #2 + live
      replay) — critère central.
- [ ] `ab_test: None` ⇒ comportement R3.S7 inchangé (non-régression).
- [ ] `clippy --all-targets` exit 0 ; build par défaut inchangé.

---

## 13. Hors scope / suites futures

| Item | Sprint | Raison |
|---|---|---|
| Significativité statistique (p-value, CI) | analyse dédiée | S8 = ségrégation descriptive (§4.6) |
| A/B Mission B / Speculative | — | pas de table de scoring (§4.7) |
| Sticky bucketing par utilisateur | post-S8 | roadmap = bucket par swarm_id (§4.2) |
| Multi-variant (A/B/C/n) | post-S8 | S8 = 2 bras |
| Label `ab_arm` métriques Prometheus | R3.S7+ | attribution primaire = SavingsReport (§1.2) |
| `to_markdown_v2` complet incluant A/B | **R3.S9** | S8 livre `ab_section_markdown`, S9 l'assemble |

---

## 14. Annexe — pourquoi ce design est chirurgical

- **Précédence explicite & sûre** (§4.1) : `request > A/B > tenant > global` —
  un caller qui épingle n'est jamais expérimenté ; un tenant hors population
  garde sa résolution R3.S5. Aucun chevauchement ambigu.
- **Zéro churn router** : l'A/B ne fait que **remplir** `req.task_class_
  scoring_table` (comme R3.S5) → le bloc router (L195) est inchangé.
- **Minimal-churn report** : l'arm vit dans `ChewingMetrics` (que `RunRecord`
  porte déjà) → `RunRecord::new` (11 args) **intact**.
- **Zéro dépendance** : `blake3` (déjà présent) pour un bucketing **uniforme**
  et **déterministe** — cohérent avec le refus de `rand` (R3.S6).
- **Config sérialisable, runtime rapide** : `ABTestConfig` (owned, serde) vs
  `ABTestRuntime` (Arc, O(1)/mission) — on n'inflige pas un clone de HashMaps
  à chaque mission A/B.
- **Honnêteté de périmètre** : `fixture_filter` au comportement live défini
  (exclusion) plutôt que dead-config ; stats descriptives plutôt que faux
  p-values — la lignée R3.S5/S6/S7.
```
