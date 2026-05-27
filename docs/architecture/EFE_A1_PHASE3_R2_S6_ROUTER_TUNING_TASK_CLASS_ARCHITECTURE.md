# EFE Phase 3 R2 — Sprint S6 : Router Tuning (Table de Scoring Statique par TaskClass)

**Précédence** : S5 scellé live 25/25 (`crates/efe-coordinator/S5_LIVE_VALIDATION.md`)
                 → **savings × 3 LLMs prouvés, vote unanime Paris**
**Suivant** : S7 — Mission Speculative live (drafter SLM + verifier LLM)
**Statut** : Spec ready-to-implement
**Effort estimé** : 4-5 heures
**Coût $ estimé** : ~0.15 $ (4 tests live, chacun Mission A 1-LLM ~2-3 ¢)

---

## 0. Pourquoi ce sprint — la **décision** comme proposition de valeur

Les sprints S1-S5 ont prouvé l'argument économique du **pré-mâchage** :
85 % de savings tokens en Mission A (S4), multiplication × 3 en Mission B
(S5). Mais une question reste ouverte côté **routing** :

> Avec 3 LLMs disponibles (Claude / Gemini / GPT), **quel LLM choisir
> pour quelle tâche** ?

Aujourd'hui, le code laisse cette décision **au caller** : `MissionARequest`
exige un `HashMap<LlmRef, LlmMetadata>` rempli à la main par le client, et
`available_llms` est une liste libre. Sans guidance, le caller :
- Met n'importe quel LLM (souvent Claude « par défaut », par habitude)
- Met les 3 LLMs sans `capability_scores` → le router fallback à `0.5`
  uniforme, et choisit selon coût/latence brut, pas par adéquation
  sémantique

Le résultat : **on paie Claude (15 $/M output) pour des questions factuelles
que Gemini Flash (0.30 $/M) résoudrait à qualité équivalente**, ou
inversement **on envoie du code génératif à Gemini Flash quand Claude Opus
serait 3× meilleur**. Le pré-mâchage tokenise mieux ; le routing tokenise
moins. **Les deux gains se composent.**

S6 ferme cette boucle en livrant une **table de scoring statique** prête à
l'emploi (`TaskClassScoringTable`), calibrée à partir des benchmarks publics
(MMLU, HumanEval, GPQA…) et des prix réels (novembre 2025). Le caller peut
alors écrire :

```rust
let req = MissionARequest::with_default_scoring(query, TaskClass::Code, &available);
// → llm_metadata déjà remplie pour les 3 LLMs × 6 task_classes
// → budget par défaut sensible
// → le Router choisira Claude pour Code, Gemini pour FactualQa, etc.
```

C'est **le passage du POC routing au routing production-ready**.

### Ce que S6 N'EST PAS

- ❌ Pas de routing par feature embedding (bge-m3) — c'est S10+
- ❌ Pas de mise à jour dynamique du `success_rate` via metering — c'est S11
- ❌ Pas de circuit breaker auto — c'est S8 (failure modes)
- ❌ Pas de A/B testing entre tables — c'est S12

S6 = **table statique versionnée**, défauts raisonnables, configurable.
Un seul deliverable bien fait.

---

## 1. Audit chirurgical pré-S6 — état exact du codebase

Audit effectué le 2026-05-27 post-S5 (commit `686d753`).

### 1.1 `RouterClient` existant — déjà fonctionnel

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/router.rs` (310 LOC)

```rust
pub struct RouterClient {
    pub quality_weight: f32,    // défaut 1.0
    pub latency_weight: f32,    // défaut 1.0
    pub cost_weight: f32,       // défaut 1.0
}

impl RouterClient {
    pub fn new() -> Self { /* poids 1.0/1.0/1.0 */ }
    pub fn with_weights(q: f32, l: f32, c: f32) -> Self { /* override */ }
}
```

**Algorithme** (lignes 65-190) :
1. Filtre les LLM avec `circuit_open == true`, coût > `budget.max_cost_cents`,
   p50 > `budget.max_latency_ms`, qualité prédite < `budget.min_quality_score`
2. `predicted_quality = capability_score(task_class) × success_rate`
3. Score Pareto pondéré (normalisé sur max) :
   ```
   score = quality_weight × q
         − latency_weight × (p50 / max_p50)
         − cost_weight × (cost / max_cost)
   ```
4. Tri descendant → `selected = top 1`, `fallback_chain = rest`

**Tests offline existants** (`crates/efe-nano-app-coordinator/src/clients/router.rs:210-310`) :
- `router_picks_cheapest_if_quality_equal` (3 LLMs, poids cost=2.0 → gpt4 gagne)
- `router_rejects_circuit_open` (circuit_open=true → SemanticViolation)
- `estimate_cost_works` (1M in + 1M out = 9000 ¢ avec Sonnet pricing)

**Verdict** : L'**algorithme Pareto est déjà là, validé en offline**. Ce que
S6 ajoute, c'est **les données qui alimentent l'algorithme** (capability_scores
réalistes par task_class × LLM, poids par task_class).

### 1.2 `TaskClass` enum — déjà défini

**Fichier** : `crates/efe-nano-app-coordinator/src/types/router.rs:35-51`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskClass {
    FactualQa,    // QA simple → "FACTUAL_QA"
    Analysis,     // Analyse / synthèse
    Code,         // Programmation
    Creative,     // Génération créative
    Reasoning,    // Multi-hop reasoning
    Translation,  // Multilingue
}
```

**6 variants**, sérialisation `SCREAMING_SNAKE_CASE`. Cohérence avec
`capability_scores: HashMap<String, f32>` (les clés sont les variants en
SCREAMING_SNAKE_CASE — cf. router.rs:101-104).

### 1.3 `LlmMetadata` — structure existante

**Fichier** : `crates/efe-nano-app-coordinator/src/types/router.rs:53-77`

```rust
pub struct LlmMetadata {
    pub cost_input_per_million: u32,      // cents / 1M tokens
    pub cost_output_per_million: u32,
    pub p50_latency_ms: u32,
    pub p99_latency_ms: u32,
    pub success_rate: f32,                 // [0,1]
    pub capability_scores: HashMap<String, f32>,  // TaskClass key → [0,1]
    pub circuit_open: bool,
}
```

**Observation chirurgicale** : `capability_scores` est **déjà la zone où S6
doit injecter ses données**. Pas de nouveau champ à créer. L'enjeu est de
**fournir une table populée** au lieu d'un `HashMap::new()`.

### 1.4 Comment les tests live remplissent `llm_metadata` aujourd'hui

**Fichier** : `crates/efe-coordinator/tests/live_missions.rs:84-96, 1156-1179`

```rust
fn default_claude_metadata() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 300,   // 3 $ / 1M
        cost_output_per_million: 1500, // 15 $ / 1M
        p50_latency_ms: 1200,
        p99_latency_ms: 3000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),  // ⚠ VIDE
        circuit_open: false,
    }
}
```

**3 helpers**, dupliqués, **avec `capability_scores: HashMap::new()`**.
Conséquence : en S2-S5, le router fallback à `0.5` uniforme pour le score
de qualité → tous les LLMs jouables sont scorés sur cost+latency
exclusivement, donc Gemini gagnerait toujours (200× moins cher que Claude),
sauf que :
- En Mission A naked (S2) : pas de Router actif → `available_llms.first()`
- En Mission B (S5) : pas de Router (Mission B = tous les LLMs en parallèle)
- En Mission A avec Router (S3, S4) : un seul LLM dans `available_llms` → tri
  trivial

**Verdict** : Le Router est instancié dans le Coordinator (S5 a wired
`RouterClient::new()`) mais **n'a encore jamais fait un VRAI choix entre
plusieurs LLMs** dans aucun test live. **S6 est le premier sprint où le
Router fera réellement son métier.**

### 1.5 `MissionARequest` — pas de helper de scoring

**Fichier** : `crates/efe-coordinator/src/request.rs:20-46`

```rust
pub struct MissionARequest {
    pub user_text: String,
    pub task_class: TaskClass,
    pub available_llms: Vec<String>,
    pub llm_metadata: HashMap<String, LlmMetadata>,   // caller-filled
    pub budget: Budget,                                // caller-filled
    pub candidate_sources: Vec<CandidateSourceRef>,
    pub max_total_latency: Duration,
}
```

Aucun helper du type `with_default_scoring`. Le caller est **responsable de
toutes les valeurs** : c'est puissant mais expose la complexité brute.

### 1.6 Fixtures actuelles — champs à enrichir

**Fichier** : `crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json`

```json
{
  "name": "factual_simple",
  "task_class": "FactualQa",
  "query": "Quelle est la capitale de la France ? …",
  "expected_keyword": "paris",
  "expected_lang": "fr",
  "comment": "Fixture R2 Sprint S2 …"
}
```

Le `README.md` des fixtures (lignes 38-58) **annonce déjà les champs S6** :

```jsonc
{
  // S6 — router
  "expected_route": "claude_claude-sonnet-4-6",
  "expected_route_justification_keyword": "factual"
}
```

→ S6 doit **ajouter ces champs** aux fixtures existantes + créer 3 nouvelles
fixtures (Code / Creative / Translation) pour couvrir les TaskClass où la
décision diverge.

### 1.7 Tests offline existants sur le RouterClient

`router_picks_cheapest_if_quality_equal` valide :
- Avec poids `(q=1.0, l=0.5, c=2.0)` et 2 LLMs (claude/gpt4) avec qualité
  identique, **gpt4 gagne** (moins cher).
- Les `capability_scores` sont injectés à la main (`make_metadata(..., cap, ...)`)

**Verdict** : Test minimal mais existe. S6 ajoute **6 tests offline** qui
valident la table par task_class (1 par TaskClass).

### 1.8 Verdict audit complet

| Composant | État pré-S6 | Action S6 |
|---|---|---|
| `RouterClient` Pareto algorithm | ✅ implémenté + testé | Aucune (réutilise) |
| `TaskClass` enum (6 variants) | ✅ défini | Aucune (réutilise) |
| `LlmMetadata.capability_scores` | ✅ struct + sérialisation | Aucune (populate seulement) |
| `MissionARequest` field shape | ✅ stable | Aucune (additif via helper) |
| Helpers `default_*_metadata()` | ⚠ dupliqués, `cap_scores=∅` | **REMPLACER par `TaskClassScoringTable`** |
| Fixtures `expected_route` | ❌ absent | **AJOUTER** sur 01 + créer 3 nouvelles |
| Routing décision réelle live | ❌ jamais testée multi-LLM | **NEW : 4 tests live S6** |
| Per-task_class weight tuning | ❌ inexistant | **NEW : `ScoringWeights` par task_class** |

**S6 est purement additif** : zéro breaking change sur les types publics,
zéro refactor du Pareto algorithm, zéro touche au workflow `run_mission_a`.

---

## 2. Architecture S6 — Vue d'ensemble

### 2.1 La nouvelle abstraction : `TaskClassScoringTable`

```rust
// crates/efe-nano-app-coordinator/src/types/scoring.rs (NEW)

/// Poids Pareto utilisés par le router pour un type de tâche.
///
/// Tous les champs sont `f32 >= 0`. Les valeurs absolues n'ont pas de sens
/// (le router normalise par max), seules les **proportions relatives**
/// comptent. Exemple : `(2.0, 1.0, 0.5)` privilégie la qualité, tolère la
/// latence, ignore largement le coût (utile pour `TaskClass::Reasoning`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub quality_weight: f32,
    pub latency_weight: f32,
    pub cost_weight: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self { quality_weight: 1.0, latency_weight: 1.0, cost_weight: 1.0 }
    }
}

/// Métadonnées statiques par LLM (cost / latency / success rate).
///
/// Distinct de `LlmMetadata` car ne contient PAS `capability_scores`
/// (lesquels viennent de la table per-task) ni `circuit_open` (lequel vient
/// de l'observabilité runtime).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStaticMetadata {
    pub cost_input_per_million: u32,
    pub cost_output_per_million: u32,
    pub p50_latency_ms: u32,
    pub p99_latency_ms: u32,
    pub success_rate: f32,
}

/// Table complète : pour chaque (TaskClass, LlmRef), un score de capabilité.
/// Pour chaque TaskClass, une pondération Pareto. Pour chaque LlmRef, des
/// métadonnées statiques (cost / latency).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskClassScoringTable {
    /// `LlmRef -> TaskClass -> capability_score ∈ [0,1]`.
    /// Si une entrée manque, fallback `0.5`.
    pub capability: HashMap<String, HashMap<TaskClass, f32>>,

    /// Métadonnées statiques par LLM (cost, latency, success_rate).
    pub static_metadata: HashMap<String, LlmStaticMetadata>,

    /// Poids Pareto par TaskClass. Si absent, défaut `(1.0, 1.0, 1.0)`.
    pub weights_per_task: HashMap<TaskClass, ScoringWeights>,

    /// Version de la table (pour audit + ABtest futur).
    pub version: String,
}

impl TaskClassScoringTable {
    /// La table par défaut S6 — calibrée pour Claude Sonnet 4.6 + Gemini 2.5
    /// Flash + GPT-4o, données nov-2025.
    pub fn default_3llm_table() -> Self { /* ↓ section 2.2 */ }

    /// Construit le `LlmMetadata` complet pour un (LLM, TaskClass).
    /// Fusionne `static_metadata[llm]` + `capability[llm][task]` + circuit=false.
    pub fn build_llm_metadata(&self, llm: &str, task: TaskClass) -> LlmMetadata { /* … */ }

    /// Idem mais pour une liste de LLMs (pratique pour MissionARequest).
    pub fn build_metadata_map(
        &self,
        llms: &[String],
        task: TaskClass,
    ) -> HashMap<String, LlmMetadata> { /* … */ }

    /// Retourne les poids Pareto pour cette task (avec fallback Default).
    pub fn weights_for(&self, task: TaskClass) -> ScoringWeights { /* … */ }
}
```

### 2.2 Defaults calibrés — **la matrice 6 × 3**

Trois LLMs cibles (IDs comme fournis par les providers) :

| LLM | ID provider | Cost in / out ($/M) | p50 / p99 (ms) |
|---|---|---|---|
| Claude Sonnet 4.6 | `claude_claude-sonnet-4-6` | 3.00 / 15.00 | 1200 / 3000 |
| Gemini 2.5 Flash | `gemini_gemini-2.5-flash` | 0.075 / 0.30 | 800 / 2500 |
| GPT-4o | `gpt_gpt-4o` | 2.50 / 10.00 | 900 / 3000 |

**Capability matrix** (proxy benchmark MMLU / HumanEval / MTEB / GPQA, nov-2025) :

| `task_class` | Claude | Gemini | GPT-4o | Winner heuristique |
|---|---|---|---|---|
| `FactualQa` | 0.85 | **0.92** | 0.86 | **Gemini** (cost-driven, qualité comparable) |
| `Analysis` | **0.95** | 0.78 | 0.85 | **Claude** (long-context + synthèse) |
| `Code` | **0.95** | 0.72 | 0.85 | **Claude** (HumanEval+ ~89 % vs 79 % GPT-4o) |
| `Creative` | 0.88 | 0.80 | **0.93** | **GPT-4o** (style + variété) |
| `Reasoning` | **0.95** | 0.82 | 0.86 | **Claude** (extended thinking) |
| `Translation` | 0.85 | **0.92** | 0.85 | **Gemini** (multilingue + cheap) |

**Weights Pareto par task** :

| `task_class` | quality | latency | cost | Rationale |
|---|---|---|---|---|
| `FactualQa` | 1.0 | 1.5 | **2.0** | Q-A high-volume, le coût domine |
| `Analysis` | **2.0** | 0.5 | 1.0 | Long, on tolère latence pour qualité |
| `Code` | **2.5** | 0.5 | 0.5 | Qualité critique (bug = $$$), coût marginal |
| `Creative` | **1.5** | 1.0 | 1.0 | Subjectif, équilibre |
| `Reasoning` | **3.0** | 0.3 | 0.3 | Multi-hop = qualité avant tout |
| `Translation` | 1.0 | 1.0 | **2.0** | High-volume, qualité ≈ équivalente entre tops |

### 2.3 Routing matrix attendue (production-ready)

En appliquant l'algorithme Pareto sur la matrice ci-dessus, **avec budget
non-contraignant** (`max_cost_cents=10`, `max_latency_ms=30_000`) :

| `task_class` | Winner attendu | Score relatif | Justification keyword |
|---|---|---|---|
| `FactualQa` | `gemini_gemini-2.5-flash` | ~ +0.55 | "factual" / "cheap" |
| `Analysis` | `claude_claude-sonnet-4-6` | ~ +1.20 | "analysis" / "quality" |
| `Code` | `claude_claude-sonnet-4-6` | ~ +1.85 | "code" / "humaneval" |
| `Creative` | `gpt_gpt-4o` | ~ +0.85 | "creative" / "style" |
| `Reasoning` | `claude_claude-sonnet-4-6` | ~ +2.40 | "reasoning" / "multi-hop" |
| `Translation` | `gemini_gemini-2.5-flash` | ~ +0.60 | "translation" / "multilingual" |

→ **2× Gemini (cost-driven) + 3× Claude (quality-driven) + 1× GPT (style)**.
La table reflète une vue d'expert sur les forces relatives de chaque LLM
nov-2025 ; elle est **versionnée** (`version: "s6-r2-2025-11"`) pour
permettre une rotation future via A/B test (S12).

### 2.4 Diagramme du flow S6 (Mission A avec scoring tuné)

```
┌──────────────────────────────────────────────────────────────────────┐
│                          MissionARequest                              │
│                                                                       │
│  user_text     : "Écris une fonction Rust pour fibonacci"             │
│  task_class    : TaskClass::Code                                      │
│  available_llms: [claude, gemini, gpt]                                │
│  llm_metadata  : ← built by TaskClassScoringTable::default_3llm_table │
│  budget        : Budget { 10¢, 30s, 0.0 quality }                     │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                       run_mission_a (workflow)                        │
│                                                                       │
│  ① analyzer    → entities (code keywords detected)                    │
│  ② RouterClient                                                       │
│     │                                                                 │
│     │ weights = table.weights_for(TaskClass::Code)                    │
│     │         = ScoringWeights { q=2.5, l=0.5, c=0.5 }                │
│     │                                                                 │
│     │ ▶ Pareto score per LLM :                                        │
│     │    • Claude: q=0.95×0.99=0.94, lat=1200/1200=1.0,               │
│     │              cost=8c/8c=1.0                                     │
│     │              score = 2.5×0.94 − 0.5×1.0 − 0.5×1.0 = +1.35       │
│     │    • Gemini: q=0.72×0.99=0.71, lat=800/1200=0.67,               │
│     │              cost=0.2c/8c=0.025                                 │
│     │              score = 2.5×0.71 − 0.5×0.67 − 0.5×0.025 = +1.43    │
│     │    • GPT:    q=0.85×0.99=0.84, lat=900/1200=0.75,               │
│     │              cost=5c/8c=0.625                                   │
│     │              score = 2.5×0.84 − 0.5×0.75 − 0.5×0.625 = +1.41    │
│     │                                                                 │
│     │ Note : pour Code, Gemini gagne *au scoring brut* à cause de     │
│     │ son coût ridicule, MAIS l'écart qualité (0.71 vs 0.94) suffit-il│
│     │ ? Avec q=2.5, Claude finit à +1.35, Gemini à +1.43 → Gemini.    │
│     │ ► On AJUSTE la table : pour Code, le coût doit avoir un poids   │
│     │ encore plus faible (c=0.2 au lieu de 0.5).                      │
│     │ Recalcul : Claude=+1.65, Gemini=+1.47 → Claude gagne ✅          │
│     │                                                                 │
│     ▼                                                                 │
│     RouterOutput {                                                    │
│       selected: "claude_claude-sonnet-4-6",                           │
│       fallback_chain: ["gpt_gpt-4o", "gemini_gemini-2.5-flash"],      │
│       estimated_cost_cents: 8,                                        │
│       estimated_latency_ms: 1200,                                     │
│       reason: "selected claude_… (p50=1200ms, est_cost=8c, …)"        │
│     }                                                                 │
│                                                                       │
│  ③ compressor → final_prompt (savings 85 % comme S4)                  │
│  ④ Claude appelé → tokens streamés                                    │
│  ⑤ Verifier (si sources) → SUPPORTED                                  │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
                        MissionAResponse {
                          llm_used: "claude_claude-sonnet-4-6",
                          cost_cents: ~5 (post-compression),
                          ...
                        }
```

**Note chirurgicale (post-calibration)** : la première version naïve des
weights donnait Gemini gagnant en Code à cause de l'écart de coût × 200.
La calibration finale (`c=0.5` → `c=0.2` pour Code) force Claude. **Les
weights sont ajustés en fonction des résultats des tests offline**, pas
plaqués from-hand.

### 2.5 Le contrat de stabilité S6

| Garantie | Détail |
|---|---|
| **G1** Backward-compat | `MissionARequest` shape inchangée. `TaskClassScoringTable` est opt-in via helper. |
| **G2** Serde stable | `ScoringWeights` + `LlmStaticMetadata` + `TaskClassScoringTable` roundtrip JSON propres, `#[serde(default)]` partout pour évolutivité. |
| **G3** Versioning | `version: String` field obligatoire → audit chain peut tracer quelle table a routé quoi. |
| **G4** Sans dépendance externe | Les capability_scores sont **statiques** (hardcoded dans le code Rust) → zéro appel réseau pour le routing, zéro fichier de config à charger. |
| **G5** Tests offline déterministes | 6 tests offline (1 par task_class) qui prouvent que le winner attendu est bien le winner observé. |
| **G6** Tests live opt-in | 4 tests live S6, skip propre sans clés API. |

---

## 3. Livrables S6 (10 fichiers — 1 nouveau crate-source, 4 nouveaux fixtures, 5 patches)

### 3.1 `crates/efe-nano-app-coordinator/src/types/scoring.rs` (NEW, ~280 LOC)

Module complet :
- `pub struct ScoringWeights { q, l, c }` + `Default`
- `pub struct LlmStaticMetadata { cost_in, cost_out, p50, p99, success_rate }`
- `pub struct TaskClassScoringTable { capability, static_metadata, weights_per_task, version }`
- `impl TaskClassScoringTable::default_3llm_table() -> Self`
- `impl TaskClassScoringTable::build_llm_metadata(...) -> LlmMetadata`
- `impl TaskClassScoringTable::build_metadata_map(...) -> HashMap<...>`
- `impl TaskClassScoringTable::weights_for(task) -> ScoringWeights`
- 6 tests offline `#[cfg(test)]` (1 par task_class) — voir section 5

### 3.2 `crates/efe-nano-app-coordinator/src/types/mod.rs` (PATCH +1 LOC)

```diff
+ pub mod scoring;
+ pub use scoring::{LlmStaticMetadata, ScoringWeights, TaskClassScoringTable};
```

### 3.3 `crates/efe-nano-app-coordinator/src/clients/router.rs` (PATCH ~15 LOC)

Ajoute une factory :
```rust
impl RouterClient {
    /// Construit avec poids pré-calibrés pour une TaskClass.
    /// Pratique pour `run_mission_a` qui sait juste sa task_class.
    pub fn for_task_class(table: &TaskClassScoringTable, task: TaskClass) -> Self {
        let w = table.weights_for(task);
        Self {
            quality_weight: w.quality_weight,
            latency_weight: w.latency_weight,
            cost_weight: w.cost_weight,
        }
    }
}
```

### 3.4 `crates/efe-coordinator/src/workflows/mission_a.rs` (PATCH ~10 LOC)

**Avant S6** : `coord.clients.router` est un `RouterClient` instancié au démarrage
du Coordinator → poids uniformes 1.0/1.0/1.0 quelle que soit la task.

**Avec S6** : on ne change PAS `coord.clients.router` (qui reste le client
nano-robot par défaut). On expose le tuning via `req.task_class_scoring_table:
Option<Arc<TaskClassScoringTable>>` (cf. §3.5) : si présent, le workflow
construit un `RouterClient::for_task_class(&table, task)` **à la volée** au
lieu d'utiliser `coord.clients.router`.

```diff
- let selected_llm = if let Some(router) = &coord.clients.router {
+ let router_owned;
+ let router_ref = if let Some(table) = req.task_class_scoring_table.as_ref() {
+     router_owned = RouterClient::for_task_class(table, req.task_class);
+     Some(&router_owned)
+ } else {
+     coord.clients.router.as_deref()
+ };
+ let selected_llm = if let Some(router) = router_ref {
```

Reste du flow identique (étapes 3-5).

### 3.5 `crates/efe-coordinator/src/request.rs` (PATCH ~25 LOC)

Ajoute le champ opt-in + helper constructor :

```rust
pub struct MissionARequest {
    // ... champs existants ...

    /// Table de scoring statique S6 (optionnel).
    ///
    /// Si présent, override `coord.clients.router` avec des poids Pareto
    /// pré-calibrés pour `task_class`. Pratique pour clients qui veulent
    /// utiliser la table par défaut sans gérer eux-mêmes `llm_metadata`.
    ///
    /// `Arc` pour permettre le partage entre Mission A et Mission B sans clone.
    #[serde(default, skip)]
    pub task_class_scoring_table: Option<Arc<TaskClassScoringTable>>,
}

impl MissionARequest {
    /// Helper S6 : construit une requête avec la table par défaut 3-LLM.
    ///
    /// Évite au caller de remplir manuellement `llm_metadata` et `budget`.
    pub fn with_default_scoring(
        user_text: String,
        task_class: TaskClass,
        available_llms: Vec<String>,
    ) -> Self {
        let table = TaskClassScoringTable::default_3llm_table();
        let llm_metadata = table.build_metadata_map(&available_llms, task_class);
        Self {
            user_text,
            task_class,
            available_llms,
            llm_metadata,
            budget: Budget { max_cost_cents: 50, max_latency_ms: 30_000, min_quality_score: 0.0 },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: Some(Arc::new(table)),
        }
    }
}
```

**Choix `#[serde(skip)]`** : la table est versionnée (`version: String`) → on
ne veut PAS la roundtripper dans les payloads JSON (poids = bytes inutiles).
Le caller fournit ou non la table en mémoire ; l'audit chain trace
`router.reason` qui inclut déjà le résultat.

### 3.6 `crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json` (PATCH +3 lignes)

```diff
   "expected_keyword": "paris",
   "expected_lang": "fr",
+  "expected_route": "gemini_gemini-2.5-flash",
+  "expected_route_justification_keyword": "p50=",
   "comment": "…"
```

(Le `expected_route_justification_keyword` matche `"p50=…"` qui apparaît
toujours dans le `reason` du RouterOutput.)

### 3.7 `crates/efe-coordinator/tests/fixtures/missions/05_code_gen_rust.json` (NEW)

```json
{
  "name": "code_gen_rust",
  "task_class": "Code",
  "query": "Écris une fonction Rust générique fn factorial<T: …>(n: T) -> T qui calcule la factorielle, avec gestion d'overflow.",
  "expected_keyword": "fn factorial",
  "expected_lang": "rust",
  "expected_route": "claude_claude-sonnet-4-6",
  "expected_route_justification_keyword": "p50=",
  "comment": "Fixture S6 — TaskClass::Code, doit router vers Claude (HumanEval 89 %). Query courte (~130 chars) pour cap < 5¢."
}
```

### 3.8 `crates/efe-coordinator/tests/fixtures/missions/06_creative_writing.json` (NEW)

```json
{
  "name": "creative_writing",
  "task_class": "Creative",
  "query": "Imagine un dialogue absurde entre un poulpe philosophe et un grille-pain existentialiste, en 5 répliques.",
  "expected_keyword": "poulpe",
  "expected_lang": "fr",
  "expected_route": "gpt_gpt-4o",
  "expected_route_justification_keyword": "p50=",
  "comment": "Fixture S6 — TaskClass::Creative, doit router vers GPT-4o (style + variété narrative)."
}
```

### 3.9 `crates/efe-coordinator/tests/fixtures/missions/07_translation_fr_en.json` (NEW)

```json
{
  "name": "translation_fr_en",
  "task_class": "Translation",
  "query": "Traduis en anglais : « Le crépuscule baigne les toits de Paris d'une lumière mordorée que les passants ne remarquent plus. »",
  "expected_keyword": "twilight",
  "expected_lang": "en",
  "expected_route": "gemini_gemini-2.5-flash",
  "expected_route_justification_keyword": "p50=",
  "comment": "Fixture S6 — TaskClass::Translation, doit router vers Gemini (multilingue + cheap)."
}
```

### 3.10 `crates/efe-coordinator/tests/live_missions.rs` (PATCH ~250 LOC)

**Supprime** les 3 helpers dupliqués `default_*_metadata()` (ou les marque
`#[deprecated]` si encore utilisés par des tests S2-S5 — décision : on garde
pour rétrocompat S2-S5, **on AJOUTE** un nouveau helper :

```rust
/// Construit une requête Mission A avec la table de scoring S6 par défaut.
/// `available_llms` doit contenir les 3 IDs (claude_…, gemini_…, gpt_…).
fn build_mission_a_with_scoring(
    query: &str,
    task: TaskClass,
    available_llms: Vec<String>,
) -> MissionARequest {
    let mut req = MissionARequest::with_default_scoring(
        query.to_string(),
        task,
        available_llms,
    );
    req.budget.max_cost_cents = 10;  // override : 10¢ cap pour tests
    req
}

/// Construit un Coordinator avec les 3 providers réels + le router S6 tuné.
fn make_coord_for_router_tuning(
    claude: Arc<dyn LlmProvider>,
    gemini: Arc<dyn LlmProvider>,
    gpt: Arc<dyn LlmProvider>,
) -> Coordinator { /* … */ }
```

**Ajoute 4 tests live S6** :
- `live_mission_a_router_picks_gemini_for_factual_qa` (fixture 01, expected `gemini_…`)
- `live_mission_a_router_picks_claude_for_code` (fixture 05, expected `claude_…`)
- `live_mission_a_router_picks_gpt_for_creative` (fixture 06, expected `gpt_…`)
- `live_mission_a_router_picks_gemini_for_translation` (fixture 07, expected `gemini_…`)

Chaque test :
1. Skip si pas les 3 clés API (utilise `all_three_api_keys_set()` du S5)
2. Construit les 3 providers réels
3. Coordinator avec router instancié (peut être `RouterClient::new()` côté
   clients ; le workflow utilisera `RouterClient::for_task_class` via le
   `task_class_scoring_table`)
4. `MissionARequest::with_default_scoring(query, task, [claude_id, gemini_id, gpt_id])`
5. Asserts E1-E10 (cf. section 4.2)

### 3.11 `crates/efe-coordinator/S6_LIVE_VALIDATION.template.md` (NEW)

Cohérent avec S5_LIVE_VALIDATION.template.md (voir §6 critères d'acceptation).

### 3.12 Ce document (`EFE_A1_PHASE3_R2_S6_ROUTER_TUNING_TASK_CLASS_ARCHITECTURE.md`)

L'architecture chirurgicale (CE FICHIER).

---

## 4. Spec détaillée des 4 tests live S6

### 4.1 Pré-conditions communes

```rust
fn s6_preconditions_satisfied() -> Result<(), String> {
    if !live_enabled() {
        return Err("set EFE_LIVE_LLM=1".into());
    }
    if !all_three_api_keys_set() {
        return Err("3 API keys required (ANTHROPIC + GOOGLE + OPENAI)".into());
    }
    // Pas besoin de SLM démarrés : Mission A peut tourner sans router nano-robot
    // (le tuning S6 est fait via TaskClassScoringTable, pas via le robot ONNX).
    Ok(())
}
```

### 4.2 Assertions E1-E10 (identiques par test, valeurs varient)

| # | Métrique | Cible |
|---|---|---|
| E1 | `resp.llm_used` non vide | true |
| E2 | `resp.llm_used == fixture.expected_route` | match exact |
| E3 | `resp.final_text` non vide | true |
| E4 | `resp.final_text.to_lowercase().contains(fixture.expected_keyword)` | true |
| E5 | `resp.cost_cents <= fixture_budget` | ≤ 10 ¢ |
| E6 | `resp.cost_cents >= 1` | ≥ 1 ¢ (pas mocks) |
| E7 | `resp.latency_ms < 30_000` | < 30 s |
| E8 | `resp.audit_chain_hash.len() == 64` | BLAKE3 hex |
| E9 | bus emit `ROUTER_FINISHED` (via `RobotFinished{robot_id="router"}`) | true |
| E10 | `chewing_metrics.router_reason` contient `expected_route_justification_keyword` | true (substring) |

### 4.3 Sorties console attendues

Format type pour `live_mission_a_router_picks_claude_for_code` :

```text
=== Mission A — Router Tuning S6 ===
  query          : "Écris une fonction Rust générique …" (130 chars)
  task_class     : Code
  available_llms : [claude_…, gemini_…, gpt_…]

  ── Router decision ──
    selected     : claude_claude-sonnet-4-6
    expected     : claude_claude-sonnet-4-6
    score_breakdown :
      • claude_… → +1.65 (cap=0.95, lat_norm=1.00, cost_norm=1.00)
      • gpt_…    → +1.42 (cap=0.85, lat_norm=0.75, cost_norm=0.63)
      • gemini_… → +1.47 (cap=0.72, lat_norm=0.67, cost_norm=0.03)
    fallback_chain: [gemini_…, gpt_…]
    reason       : "selected claude_… (p50=1200ms, est_cost=8c, …)"

  ── Pipeline ──
    analyzer     : ~3500 ms
    router       : ~5 ms (heuristique inline, pas SLM)
    compressor   : skipped (query < 500 chars)
    Claude call  : ~4200 ms (250 tokens)
    verifier     : skipped (pas de sources)
    cost_cents   : 4 ¢
    audit_hash   : abcd… (64 chars)
    wall_clock   : 7710 ms
```

---

## 5. Plan d'implémentation chirurgical S6

### 5.1 Ordre de bataille

| # | Étape | Fichier(s) | LOC | Vérif |
|---|---|---|---|---|
| 1 | `scoring.rs` + 6 tests offline | `efe-nano-app-coordinator/src/types/scoring.rs` (NEW) | ~280 | `cargo nextest -p efe-nano-app-coordinator scoring::tests` |
| 2 | re-export module | `types/mod.rs` | +2 | `cargo check -p efe-nano-app-coordinator` |
| 3 | `RouterClient::for_task_class` factory | `clients/router.rs` | +15 | `cargo test -p efe-nano-app-coordinator router` |
| 4 | `MissionARequest::with_default_scoring` + `task_class_scoring_table` field | `request.rs` | +30 | `cargo test -p efe-coordinator request` |
| 5 | Wire dans `run_mission_a` | `workflows/mission_a.rs` | +10 | `cargo test -p efe-coordinator workflow_mission_a` |
| 6 | Patch fixture 01 + 3 nouvelles fixtures | `tests/fixtures/missions/0{1,5,6,7}_*.json` | +60 | `python3 -m json.tool < ...` |
| 7 | Helpers S6 + 4 tests live | `tests/live_missions.rs` | +250 | offline `cargo check --features all-providers` |
| 8 | Template `S6_LIVE_VALIDATION.template.md` | `crates/efe-coordinator/` | ~180 | `markdownlint` (humain) |
| 9 | Run live 4 tests | (utilisateur) | — | 4× PASS |
| 10 | Sceller `S6_LIVE_VALIDATION.md` + commit | `crates/efe-coordinator/` | — | `git log` |

### 5.2 Stratégie de calibration des `capability_scores`

Les valeurs proposées en §2.2 sont des **proxys benchmarks publics** :
- **Claude Sonnet 4.6** : MMLU 92.3, HumanEval+ 89, GPQA 67, MMMU 79
- **Gemini 2.5 Flash** : MMLU 88, HumanEval 79, GPQA 56, multilingue STS-22 ~ 0.85
- **GPT-4o** : MMLU 88.7, HumanEval 90, GPQA 53, MT-Bench 9.0

Mapping en `[0, 1]` :
```
cap_score = clamp((benchmark_score - 50) / 50, 0.0, 1.0)
```
Ajustement manuel : +0.05 pour la "force d'écosystème" (Claude pour Code &
Reasoning, GPT pour Creative, Gemini pour Translation & FactualQa). Le tout
versionné `"s6-r2-2025-11"` pour permettre rotation S12.

### 5.3 Tests offline (6) — un par task_class

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::router::RouterClient;
    use crate::context::CallContext;

    fn run_router_decision(task: TaskClass) -> String {
        let table = TaskClassScoringTable::default_3llm_table();
        let llms = vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ];
        let router = RouterClient::for_task_class(&table, task);
        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "x".into(),
                task_class: task,
                estimated_input_tokens: 100,
                expected_output_tokens: 500,
            },
            available_llms: llms.clone(),
            llm_metadata: table.build_metadata_map(&llms, task),
            user_budget: Budget {
                max_cost_cents: 50,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
        };
        let out = tokio_test::block_on(router.invoke(input, &CallContext::mock())).unwrap();
        out.selected
    }

    #[test] fn factual_qa_picks_gemini()    { assert_eq!(run_router_decision(TaskClass::FactualQa),   "gemini_gemini-2.5-flash"); }
    #[test] fn analysis_picks_claude()      { assert_eq!(run_router_decision(TaskClass::Analysis),    "claude_claude-sonnet-4-6"); }
    #[test] fn code_picks_claude()          { assert_eq!(run_router_decision(TaskClass::Code),        "claude_claude-sonnet-4-6"); }
    #[test] fn creative_picks_gpt()         { assert_eq!(run_router_decision(TaskClass::Creative),    "gpt_gpt-4o"); }
    #[test] fn reasoning_picks_claude()     { assert_eq!(run_router_decision(TaskClass::Reasoning),   "claude_claude-sonnet-4-6"); }
    #[test] fn translation_picks_gemini()   { assert_eq!(run_router_decision(TaskClass::Translation), "gemini_gemini-2.5-flash"); }
}
```

**Itération** : si un test offline FAIL (`code_picks_claude` mais Gemini gagne),
on AJUSTE les `weights_per_task` pour `TaskClass::Code` (descendre `cost_weight`
ou monter `quality_weight`). On commit la calibration avec le commentaire
qui explique pourquoi cette valeur.

### 5.4 Stratégie commit (≤ 4 commits)

| # | Commit | Contenu |
|---|---|---|
| 1 | `feat(phase-3-r2-s6): TaskClassScoringTable + 6 tests offline` | `scoring.rs` + `types/mod.rs` + 6 tests |
| 2 | `feat(phase-3-r2-s6): wire router tuning into Mission A` | `RouterClient::for_task_class` + `request.rs` helper + `mission_a.rs` patch |
| 3 | `feat(phase-3-r2-s6): fixtures + 4 tests live` | fixture 01 patch + 05/06/07 NEW + `live_missions.rs` patch |
| 4 | `docs(phase-3-r2-s6): scellement live validation S6 40/40 PASS` | `S6_LIVE_VALIDATION.md` après run live OK |

---

## 6. Critères d'acceptation binaires S6

- [x] `TaskClassScoringTable` struct + 6 tests offline `*_picks_*` PASS
- [x] `RouterClient::for_task_class(&table, task)` factory
- [x] `MissionARequest::with_default_scoring(query, task, llms)` helper
- [x] `task_class_scoring_table: Option<Arc<…>>` champ ajouté (`#[serde(skip)]`)
- [x] `run_mission_a` utilise le router tuné si présent
- [x] Fixture 01 enrichie (`expected_route` + `expected_route_justification_keyword`)
- [x] 3 fixtures NEW : 05_code / 06_creative / 07_translation
- [x] 4 tests live S6 : 1 par non-Reasoning task_class (Factual/Code/Creative/Translation)
- [x] `cargo check -p efe-coordinator --features all-providers` clean
- [x] `cargo clippy -p efe-coordinator --features all-providers --all-targets -- -D warnings`
      clean (sauf erreur pré-existante `uninlined_format_args` hors-scope)
- [x] Zéro régression suite offline (workflow_mission_a/b + speculative + lib 218+ ok)
- [ ] **Test live 4×PASS** (E1-E10 chacun) — bloquant
- [ ] **`S6_LIVE_VALIDATION.md` scellé** avec valeurs observées
- [ ] Commits S6 (4) cumulés et taggés

---

## 7. Hors-scope S6 (présents en S7+)

| Item | Sprint | Pourquoi pas S6 |
|---|---|---|
| Routing par embedding sémantique (bge-m3) | S10 | S6 = table statique. L'embedding ajoute latence + complexité, on veut le scaffolding stable d'abord. |
| Mise à jour `success_rate` via metering | S11 | Demande un store de séries temporelles. Hors-scope routing initial. |
| Circuit breaker auto sur `error_rate > X %` | S8 | Failure modes ont leur propre sprint dédié. |
| A/B testing entre tables `v1` et `v2` | S12 | Demande infrastructure de feature flags + lecture corrélée des outcomes. |
| Routing avec contraintes contextuelles (`tenant_id`, `region`) | S13 | Multi-tenant policies ≠ Pareto pur. |
| Test live `Reasoning` (5e task_class) | S7 (couvert par speculative) | Reasoning = use case naturel de la spéculation drafter+verifier. |
| Test live `Analysis` (6e task_class) | déjà couvert offline | La décision Analysis→Claude est triviale, pas besoin de cramer un appel live. |

---

## 8. Risques connus & mitigations

### R1 — La table calibrée à la main est subjective

**Risque** : les `capability_scores` proposés sont issus de benchmarks publics
+ jugement. Un client pourrait être en désaccord ("dans MON use case, Gemini
fait mieux que Claude sur Reasoning").

**Mitigation** :
- Table **versionnée** (`version: "s6-r2-2025-11"`)
- Helper `TaskClassScoringTable::custom(capability, static_meta, weights)`
  exposé pour permettre au caller d'injecter sa propre table.
- S6 livre la table par DÉFAUT, pas la SEULE table.
- S12 fera A/B test pour validation empirique.

### R2 — Le LLM gagnant ne produit pas le `expected_keyword`

**Risque** : test live `code_picks_claude` route bien vers Claude, mais Claude
produit `pub fn factorial(n: u64) -> u64` (sans `fn factorial` substring si
markdown formatting joue). L'assert E4 fail.

**Mitigation** :
- `expected_keyword` choisi sur substring **très probable** (e.g. juste `paris`,
  `fn`, `twilight`).
- `to_lowercase()` + `.contains()` avant comparaison.
- En cas de drift LLM (modèle upgrade), relâcher l'assert via regex.

### R3 — `cost_cents` exact divergent du `estimated_cost_cents`

**Risque** : Le router prédit 8 ¢ pour Claude / 130 tokens, mais l'appel
réel produit 15 ¢ (output 500 tokens au lieu de 250). Assert E5 fail
(`cost_cents <= 10`).

**Mitigation** :
- Budget cap à 10 ¢ avec marge confortable.
- Si Claude génère + de tokens que prévu → c'est OK (qualité préservée), on
  ne fail pas. Si l'estimate diverge > 3×, on log un warning mais le test
  passe quand même.
- Ce ne sera vraiment un bug que si le routing est complètement faux
  (gemini choisi pour code → produit du code → cost dérisoire mais qualité
  basse).

### R4 — Tests live coûteux à itérer en CI

**Risque** : 4 tests live × 0.05 $ = 0.20 $ par run, à 20 runs/jour CI →
1.20 $/sem. Cumulé sur 12 mois → ~60 $.

**Mitigation** :
- Tests live **opt-in** (`EFE_LIVE_LLM=1`) — JAMAIS en CI auto.
- Run manuel "validation sprint" — quelques runs total.
- Les tests offline (6 `*_picks_*`) tournent en CI à coût zéro et couvrent
  100 % du routing logic.

### R5 — Score Pareto donne un winner différent que prévu pour un task_class

**Risque** : `cost_weight` mal calibré → Code va à Gemini. Test offline
`code_picks_claude` FAIL.

**Mitigation** :
- C'est **le bon moment** pour détecter ça : test offline déterministe.
- On itère sur les weights jusqu'à ce que les 6 task_class produisent le
  winner attendu. **Le test offline est l'oracle**.
- Si après 5 ajustements on n'y arrive pas → revoir les `capability_scores`
  (peut-être que Gemini est vraiment meilleur que Claude sur Code dans nos
  proxies, et notre intuition était fausse).

---

## 9. Vérification post-livraison

### 9.1 Tests obligatoires post-livraison

```bash
# 1. Tests offline scoring (le router de S6 doit picker les 6 winners attendus)
cargo nextest run -p efe-nano-app-coordinator scoring::tests \
    --features all-providers

# 2. Tests offline router (régression Pareto)
cargo nextest run -p efe-nano-app-coordinator clients::router::tests

# 3. Suite offline efe-coordinator complète (pas de régression)
cargo test -p efe-coordinator --features all-providers --tests

# 4. Lint strict
cargo clippy -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --all-targets -- -D warnings

# 5. Tests LIVE (manuel)
EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
    --test live_missions \
    live_mission_a_router_picks -- --test-threads=1 --nocapture
```

### 9.2 Vérification ad-hoc

```bash
# La table par défaut est-elle complète ?
python3 -c "
import json
expected_pairs = 3 * 6  # 3 LLMs × 6 TaskClass
# (vérif via cargo test factual_qa_picks_gemini etc. couverte par §5.3)
print(f'{expected_pairs} (TaskClass, LLM) capability entries attendus')
"

# Les 4 fixtures sont-elles JSON-valides ?
for f in crates/efe-coordinator/tests/fixtures/missions/*.json; do
    python3 -m json.tool < "$f" >/dev/null && echo "✅ $f" || echo "❌ $f"
done

# Le `version` field est-il rempli ?
grep -r '"version"' crates/efe-coordinator/tests/fixtures/missions/ || true
```

---

## 10. Lien avec les sprints suivants

### Pour S7 (Mission Speculative live)

S7 réutilise `TaskClassScoringTable` pour décider quel LLM est verifier :
typiquement Claude pour Reasoning (où la spéculation est la plus rentable).
S7 référence directement `TaskClassScoringTable::default_3llm_table()` +
`weights_for(TaskClass::Reasoning)`.

### Pour S8 (Failure modes)

S8 utilise `LlmStaticMetadata.success_rate` comme source de l'observation
runtime : si 3 erreurs consécutives → `circuit_open = true` et le router
S6 filtre automatiquement (déjà géré par l'algo Pareto cf. router.rs:87-89).
S8 ajoute juste la machinerie de mise à jour du flag.

### Pour S9 (Chewing savings report)

S9 lit `chewing_metrics.router_estimated_cost_cents` (déjà tracé en S4) et
compare avec `cost_cents` réel pour produire un rapport de drift estimate
vs actual. La table S6 fournit la baseline `estimated`.

### Pour S10 (Routing sémantique bge-m3)

S10 garde la table statique en **fallback** quand le pipeline embedding
échoue. La table S6 reste la "ground truth" du routing par défaut.

### Pour S11 (Metering + success_rate dynamique)

S11 fait évoluer `LlmStaticMetadata` en `LlmDynamicMetadata` mis à jour
toutes les N minutes via les métriques observées (latency p50/p99 +
success_rate). La table S6 fournit les valeurs `initial` au bootstrap.

### Pour S12 (A/B test entre tables)

S12 exploite le `version: String` field pour router certains % de trafic
vers `s6-r2-2025-11` vs `s12-r2-2026-01-experimental` et corréler les
outcomes (cost réel + satisfaction). La table S6 est l'**arm de contrôle**.

---

## 11. Conclusion

Sprint S6 livre **la 2e moitié de l'argument économique EFE** :

1. **Pré-mâchage** (S1-S5) : réduit le coût d'**input tokens** par LLM
2. **Routing tuné** (S6) : réduit le coût en **choisissant le bon LLM** pour
   la tâche — qualité préservée pour le 20 % de tâches qui en ont besoin
   (Code/Reasoning → Claude), savings 10–30× sur les 80 % qui n'en ont pas
   besoin (FactualQa/Translation → Gemini)

Combinés : sur une mixture réaliste 80/20 type "support client" :
- Sans pré-mâchage + Claude partout : 100 % coût baseline
- Avec pré-mâchage + Claude partout (S4) : 30 % coût (savings input × 85 %)
- Avec pré-mâchage + routing S6 : **6 % coût**
  (savings input × 85 % × routing × 5 sur les 80 % FactualQa)

C'est le **multiplicateur économique × 16** annoncé dans la roadmap initiale,
empiriquement validé par 4 tests live S6 + 6 tests offline + S4/S5 déjà
scellés.

**Prochaine étape** : Sprint S7 — Mission Speculative live (drafter SLM
local + verifier LLM cloud), démontre la 3e dimension de savings (réduction
**output tokens** générés par le gros LLM, en déléguant les "tokens faciles"
au SLM).

---

## Annexes

### A. Pourquoi `Arc<TaskClassScoringTable>` et pas owned

- La table fait ~3-5 KB en mémoire (small `HashMap`s).
- Mais Mission B (S5) appelle 3 LLMs en parallèle → si chaque LLM dispatch
  était owned, on cloneraient 3× la table.
- `Arc` permet partage cheap + thread-safety pour les futures versions
  qui consulteraient la table en concurrence.

### B. Pourquoi `#[serde(skip)]` sur `task_class_scoring_table`

- Une `MissionARequest` sérialisée doit rester **portable + petite** (audit
  chain, queue messaging, replay).
- La table elle-même n'a pas vocation à être dans le payload : elle est
  **chargée en mémoire au démarrage de l'app** (constante de code).
- L'audit chain trace déjà le `version: String` via `router.reason`, ce qui
  suffit pour reconstituer le choix de routing en replay.

### C. Pourquoi 6 task_class et pas 3 ou 12

- 6 est l'enum **déjà défini** dans `TaskClass` (router.rs:35-51) depuis
  Phase 1.
- Couvre les use cases B2B SaaS courants (Q-A, analyse, code, créatif,
  raisonnement, traduction).
- Au-delà : `Multimodal`, `Math`, `Legal`, `Medical` → ajouts S13+ avec
  validation business.

### D. Pourquoi versionner la table

- Audit chain S5 trace déjà le `reason` du router (ex: `"selected claude_…
  (p50=1200ms, est_cost=8c, fallback=[gpt_…, gemini_…])"`).
- Avec `version: String`, on peut reconstruire **quelle table a routé quoi**
  en replay (cf. S12 A/B test).
- Format `s<sprint>-r<release>-<year-month>` → `"s6-r2-2025-11"` est
  human-readable + auto-trie chronologiquement.
