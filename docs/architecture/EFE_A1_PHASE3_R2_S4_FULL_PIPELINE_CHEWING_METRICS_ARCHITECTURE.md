# EFE Phase 3 R2 — Sprint S4 : Pipeline Pré-Mâchage Complet + ChewingMetrics

**Précédence** : S3 scellé live 16/16 (`crates/efe-coordinator/S3_LIVE_VALIDATION.md`)
**Suivant** : S5 — Mission B (3-LLM vote)
**Statut** : Spec ready-to-implement
**Effort estimé** : 6 heures (le plus gros sprint de R2)
**Coût $ estimé** : ~0.05 $ (Claude × 3-5, SLM gratuits)

---

## 0. Pourquoi ce sprint, dans cet ordre

> **C'est LE sprint qui prouve enfin le but premier de R2 : économiser des
> tokens sur les gros LLMs grâce au pré-mâchage SLM.**

S2-S3 ont prouvé que la collaboration SLM ↔ LLM marche. S4 ajoute la mesure
**objective et quantifiée** du gain :

- **compressor** (Llama-3.2-1B :8092) — *RÉDUIT* les tokens d'entrée du gros LLM
  via résumé contraint
- **router** (heuristique Pareto in-process) — *CHOISIT* le LLM optimal selon
  le triptyque cost/quality/latency
- **`ChewingMetrics`** — module nouveau qui *MESURE* tokens_before /
  tokens_after / compression_ratio / savings_$
- **`StreamEvent::PreChewingMetrics`** — variant SSE nouveau qui expose les
  métriques au consommateur (UI ou test)

Après S4, on aura un **chiffre dur** du type : « pour cette query de 200
tokens, on a fait passer le gros LLM sur 80 tokens compressés, savings 60 %,
verdict toujours SUPPORTED ».

C'est l'argument économique central qui justifie EFE vs un appel naïf Claude.

---

## 1. Audit chirurgical pré-S4 — état exact du codebase

Audit complet effectué le 2026-05-27. **Découverte critique** ci-après.

### 1.1 `CompressorClient` — API

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/compressor.rs:22-94`

```rust
pub struct CompressorClient { /* inner: LlamaServerClient */ }

impl CompressorClient {
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self>;
    // → port 8092, max_tokens=1536, temperature=0.2
}

// trait NanoRobotClient :
async fn invoke(&self, input: CompressorInput, ctx: &CallContext)
    -> RobotResult<CompressorOutput>;
async fn health(&self) -> RobotResult<RobotHealth>;
```

**Types** :

```rust
pub struct CompressorInput {
    pub text: String,                    // 50 ≤ len ≤ 50_000
    pub target_words: u32,               // 10 ≤ n ≤ 500
    pub must_preserve: Vec<String>,      // ≤ 50 entités obligatoires
    pub style: CompressionStyle,         // enum: Concise | Bullet | Telegraphic
}

pub struct CompressorOutput {
    pub summary: String,                 // 1 ≤ len ≤ 2000
    pub word_count: u32,                 // 1 ≤ n ≤ 500
    pub preserved: Vec<String>,          // ≤ 50 entités effectivement préservées
    pub omitted_for_brevity: Vec<String>,// ≤ 20 snippets sacrifiés
    pub compression_ratio: f32,          // 1.0 ≤ r ≤ 50.0 (source_words / summary_words)
}
```

**Point clé** : `compression_ratio` est **natif dans l'output**. Pas besoin
de le recalculer dans `ChewingMetrics` — il faut juste l'exposer.

### 1.2 `RouterClient` — API heuristique (PAS llama-server, PAS HTTP)

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/router.rs:27-152`

```rust
pub struct RouterClient {
    quality_weight: f32,
    latency_weight: f32,
    cost_weight: f32,
}

impl RouterClient {
    pub fn new() -> Self;                                            // weights = 1.0 chacun
    pub fn with_weights(quality: f32, latency: f32, cost: f32) -> Self;
}

// Pas de from_assets, pas d'endpoint HTTP — instancié inline en Rust.
```

**Types** :

```rust
pub struct RouterInput {
    pub task: TaskDescription,                     // depuis analyzer + user_text
    pub available_llms: Vec<LlmRef>,              // 1..10 candidats
    pub llm_metadata: HashMap<LlmRef, LlmMetadata>,
    pub user_budget: Budget,                       // depuis MissionARequest.budget
}

pub struct RouterOutput {
    pub selected: LlmRef,                          // l'ID du LLM gagnant
    pub fallback_chain: Vec<LlmRef>,               // ordre de retry si selected fail
    pub estimated_cost_cents: u32,                 // estimation router
    pub estimated_latency_ms: u32,
    pub reason: String,                            // debug/audit (≤ 200 chars)
}
```

**Algorithme** : `score = quality_weight × q − latency_weight × norm(lat) −
cost_weight × norm(cost)`. Pareto multi-critères sur la table fournie.

### 1.3 `ReformulatorClient` + `FilterClient` — **non utilisés dans le workflow**

**Découverte critique** : `crates/efe-coordinator/src/workflows/mission_a.rs`
contient l'ordre exact suivant :

| Étape | Lignes | Robot |
|---|---|---|
| 1 | 63-129 | **analyzer** |
| 2 | 132-200 | **router** |
| 3 | 203-262 | **compressor** (skip si len < 500 chars) |
| 4 | 264-305 | **LLM call** streaming |
| 5 | 307-352 | **verifier** (skip si pas de sources) |

**`reformulator` et `filter` sont absents.** Bien que les champs
`CoordinatorClients.reformulator` et `.filter` existent, le workflow
`run_mission_a` ne les invoque jamais.

**Implication pour S4** : on **ne peut pas** tester "le pipeline complet
6 robots" comme prévu dans l'architecture R2 initiale (section 2.2). On
teste le pipeline **réel** : 4 robots actifs (analyzer + router + compressor
+ verifier). Reformulator + filter restent **inutilisés** jusqu'à
modification du workflow source (potentiellement S5 ou Phase 4).

**Décision** : on accepte cette réalité. S4 valide les 4 robots déjà
intégrés + `ChewingMetrics` qui mesure le gain de compression et le choix
du router. Reformulator + filter seront **mentionnés dans le doc S4
comme "wired mais inutilisés — à intégrer en S5+"**.

### 1.4 Comment `run_mission_a` orchestre compressor (lignes 203-262)

```rust
// Phase 3 — Compressor (skip si user_text < 500 chars)
let final_prompt = if let Some(compressor) = &coord.clients.compressor {
    if req.user_text.len() >= 500 {   // ← seuil hardcodé
        let input = CompressorInput {
            text: req.user_text.clone(),
            target_words: 100,             // ← hardcodé
            must_preserve: extract_must_preserve_from_blackboard(coord),
            style: CompressionStyle::Concise,
        };
        let out = compressor.invoke(input, &ctx).await?;
        // Bus event : ROBOT_FINISHED("compressor")
        // BlackboardEntry::new("compressor", EntryKind::Summary, json!({...}))
        out.summary  // ← le summary remplace user_text
    } else {
        req.user_text.clone()  // texte court : passthrough
    }
} else {
    req.user_text.clone()  // compressor absent : passthrough
};
// final_prompt est ce qui est envoyé au LLM
```

**Conséquences importantes pour les fixtures S4** :
- La query de S2/S3 ("Quelle est la capitale...") fait **76 caractères** → **bypass compressor** (seuil 500).
- Pour activer le compressor, on a besoin d'une fixture avec un `user_text` ≥ 500 chars.
- On va créer `03_factual_compressible.json` avec un paragraphe long.

### 1.5 Comment `run_mission_a` orchestre router (lignes 132-200)

```rust
let selected_llm = if let Some(router) = &coord.clients.router {
    let input = RouterInput {
        task: TaskDescription { ... extracted from analyzer output },
        available_llms: req.available_llms.clone(),
        llm_metadata: req.llm_metadata.clone(),
        user_budget: req.budget.clone(),
    };
    let out = router.invoke(input, &ctx).await?;
    out.selected  // ← LLM choisi par Pareto
} else {
    // Fallback : 1er de la liste (= S2/S3 path)
    req.available_llms.first().cloned().ok_or(...)?
};
```

### 1.6 Module `metrics` — **ABSENT**

**Aucun `metrics.rs` ou module `metrics/` dans le repo.** S4 doit le créer
`ex nihilo` à : `crates/efe-coordinator/src/metrics.rs`.

### 1.7 Tokenisation actuelle — heuristique 1 token ≈ 4 chars

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/filter.rs:327`

```rust
fn estimate_tokens(text: &str) -> u32 {
    ((text.chars().count() as f32 / 4.0).ceil()) as u32
}
```

**Pas de tiktoken**. L'heuristique 1 token = 4 chars est utilisée partout
dans le projet. Pour `ChewingMetrics`, on utilise la même convention :
**zéro nouvelle dépendance**.

**Alternative** : Claude retourne `LlmResponse.tokens_generated` (vrais
tokens via API). On peut donc avoir une vérité pour l'output. Pour l'input
on garde l'heuristique.

### 1.8 `StreamEvent` enum — pattern d'ajout d'un variant

**Fichier** : `crates/efe-coordinator/src/events/stream.rs:38-195`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamEvent {
    SwarmStarted { swarm_id: Uuid, ts: DateTime<Utc>, ... },
    SwarmFinished { swarm_id: Uuid, ... },
    RobotStarted { swarm_id: Uuid, robot_id: String, ts: DateTime<Utc> },
    RobotFinished { swarm_id: Uuid, robot_id: String, success: bool,
                    latency_ms: u64, error: Option<String>, ts: DateTime<Utc> },
    PheromoneDeposited { ... },
    LlmStarted { ... },
    LlmTokenStreamed { ... },
    LlmFinished { ... },
    VerdictReady { ... },
    // … autres variants
}

impl StreamEvent {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::SwarmStarted { .. } => "SWARM_STARTED",
            // …
        }
    }
}
```

**Ajout S4 (proposition)** :

```rust
PreChewingMetrics {
    swarm_id: Uuid,
    metrics: ChewingMetrics,   // struct serde de notre nouveau module
    ts: DateTime<Utc>,
},

// Et dans tag() :
Self::PreChewingMetrics { .. } => "PRE_CHEWING_METRICS",
```

Le rename serde `SCREAMING_SNAKE_CASE` garantit le JSON `"type": "PRE_CHEWING_METRICS"`.

### 1.9 `MissionAResponse` — extension via `#[serde(default)]`

```rust
pub struct MissionAResponse {
    pub final_text: String,
    pub llm_used: String,
    pub cost_cents: u32,
    pub latency_ms: u64,
    pub verdicts: Vec<FactCheck>,
    pub audit_chain_hash: String,
    // ┌── S4 add ──────────────────────────────────────┐
    // │ #[serde(default)]                              │
    // │ pub chewing_metrics: Option<ChewingMetrics>,   │
    // └────────────────────────────────────────────────┘
}
```

**Backward compat** : `#[serde(default)]` garantit que les `MissionAResponse`
sérialisés sans ce champ continuent de désérialiser correctement (Option = None).

### 1.10 Verdict audit pré-S4

| Composant | Statut | Action S4 |
|---|---|---|
| `CompressorClient` | ✅ existant | Brancher dans test |
| `RouterClient` | ✅ existant | Brancher dans test |
| `ReformulatorClient` | ⚠️ existant **mais non utilisé** | Hors-scope S4 |
| `FilterClient` | ⚠️ existant **mais non utilisé** | Hors-scope S4 |
| `compressor` dans workflow | ✅ branché lignes 203-262 | OK |
| `router` dans workflow | ✅ branché lignes 132-200 | OK |
| `reformulator` dans workflow | ❌ pas appelé | Mention seulement |
| `filter` dans workflow | ❌ pas appelé | Mention seulement |
| Module `metrics` | ❌ inexistant | **À créer** |
| `StreamEvent::PreChewingMetrics` | ❌ inexistant | **À ajouter** |
| `MissionAResponse.chewing_metrics` | ❌ inexistant | **À ajouter (optionnel)** |
| Émission event dans `run_mission_a` | ❌ inexistant | **À ajouter** |

**Conclusion** : S4 = **première modification non triviale du code source** de
R2 (S2 et S3 n'ont modifié AUCUN fichier .rs source). On ajoute 1 module
nouveau (`metrics.rs`), 1 variant enum, 1 champ optionnel à un struct, et
~30 lignes dans `mission_a.rs` pour mesurer + émettre l'event final.

---

## 2. Architecture S4 — Vue d'ensemble

### 2.1 Flow Mission A complète (4 robots actifs)

```text
   ┌──────────────────────────────────────────────────────────────────────┐
   │  TEST live_mission_a_full_pre_chewing_pipeline                       │
   │  EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1 + ANTHROPIC_API_KEY                  │
   │  + llama-server :8090 + :8092 + :8093 UP (analyzer/compressor/verifier)│
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  Coordinator avec 4 nano-robots + Claude :                           │
   │  clients = {                                                          │
   │    analyzer:     Some(:8090),   verifier: Some(:8093),               │
   │    compressor:   Some(:8092),                                        │
   │    router:       Some(RouterClient::new()),  // in-process           │
   │    reformulator: None,          filter:   None,  // S5+              │
   │  }                                                                    │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionARequest {                                                   │
   │    user_text          : "<paragraphe long ≥ 500 chars>"              │
   │    available_llms     : vec![claude_id]                              │
   │    candidate_sources  : vec![<source 1>, <source 2>]                 │
   │  }                                                                    │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  run_mission_a(&coord, req).await — modifié S4 :                     │
   │                                                                       │
   │  let t_start = Instant::now();                                       │
   │  let mut metrics = ChewingMetrics::new();                            │
   │  metrics.input_tokens_raw = estimate(req.user_text);                 │
   │                                                                       │
   │  ── 1. SWARM_STARTED ──                                              │
   │                                                                       │
   │  ── 2. ANALYZER ──                                                   │
   │  let t_a = Instant::now();                                           │
   │  analyzer.invoke(...).await?;                                        │
   │  metrics.analyzer_latency_ms = t_a.elapsed();                        │
   │  (events : ROBOT_STARTED, PHEROMONE_DEPOSITED ×N, ROBOT_FINISHED)    │
   │                                                                       │
   │  ── 3. ROUTER (heuristique Pareto, in-process) ──                    │
   │  let t_r = Instant::now();                                           │
   │  let routing = router.invoke(...).await?;                            │
   │  metrics.router_latency_ms = t_r.elapsed();                          │
   │  metrics.selected_llm = routing.selected;                            │
   │  metrics.router_reason = routing.reason;                             │
   │                                                                       │
   │  ── 4. COMPRESSOR (skip si len < 500) ──                             │
   │  let final_prompt = if req.user_text.len() >= 500 {                  │
   │      let t_c = Instant::now();                                       │
   │      let out = compressor.invoke(...).await?;                        │
   │      metrics.compressor_latency_ms = t_c.elapsed();                  │
   │      metrics.compression_ratio = 1.0 / out.compression_ratio;        │
   │      metrics.input_tokens_compressed = estimate(out.summary);        │
   │      out.summary                                                      │
   │  } else {                                                             │
   │      metrics.input_tokens_compressed = metrics.input_tokens_raw;     │
   │      metrics.compression_ratio = 1.0;                                │
   │      req.user_text.clone()                                           │
   │  };                                                                   │
   │                                                                       │
   │  ── 5. LLM CALL ──                                                   │
   │  let t_l = Instant::now();                                           │
   │  let llm_resp = bridge.stream_one(&selected_llm, &final_prompt)     │
   │                       .await?;                                       │
   │  metrics.llm_latency_ms = t_l.elapsed();                             │
   │  metrics.llm_tokens_generated = llm_resp.tokens_generated;           │
   │                                                                       │
   │  ── 6. VERIFIER ──                                                   │
   │  let t_v = Instant::now();                                           │
   │  verifier.invoke(...).await?;                                        │
   │  metrics.verifier_latency_ms = t_v.elapsed();                        │
   │                                                                       │
   │  ── 7. CALCULS FINAUX ──                                             │
   │  metrics.savings_tokens = raw - compressed;                          │
   │  metrics.savings_ratio = 1.0 - compression_ratio;                    │
   │  metrics.total_slm_latency_ms = analyzer + compressor + verifier;    │
   │  metrics.total_pipeline_latency_ms = t_start.elapsed();              │
   │                                                                       │
   │  ── 8. ÉMISSION ÉVÉNEMENT FINAL ──                                  │
   │  bus.publish(PreChewingMetrics { swarm_id, metrics, ts }).await;     │
   │                                                                       │
   │  ── 9. SWARM_FINISHED ──                                             │
   │                                                                       │
   │  Ok(MissionAResponse {                                               │
   │    final_text, llm_used, cost_cents, latency_ms, verdicts,           │
   │    audit_chain_hash,                                                 │
   │    chewing_metrics: Some(metrics),  // S4 NEW                        │
   │  })                                                                   │
   └──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Latence cumulée attendue

| Phase | Estimation (Mac M-series) |
|---|---|
| Pre-flight 3 SLM health | ~70 ms |
| analyzer (cold) | ~5-7 s (20 entités) |
| router (in-process Rust) | < 5 ms |
| compressor (~500 chars → 100 mots) | ~3-5 s (Llama-3.2-1B) |
| Claude HTTPS (sur prompt court compressé) | ~1-2 s (économie tokens) |
| verifier | ~3-4 s |
| Overhead Coordinator | ~50 ms |
| **Total cold** | **~13-18 s** |
| **Total warm (run #2)** | **~7-10 s** |

### 2.3 Coût attendu

| Item | Coût |
|---|---|
| Claude Sonnet 4.6 sur prompt **compressé** (~30 tokens) | ~0.5 ¢ |
| Analyzer/Compressor/Verifier (local Metal) | 0 ¢ |
| Router (in-process Rust) | 0 ¢ |
| **Total** | **~1 ¢** |

vs **baseline naïf** (envoyer le paragraphe long de 500 chars = ~125 tokens à
Claude) : ~2-3 ¢. **Savings : 50-66 %.**

### 2.4 Périmètre exact S4 (par rapport à l'architecture R2 globale)

| Composant promis en R2 | Statut S4 |
|---|---|
| analyzer (intent + entités) | ✅ déjà branché S3 |
| verifier (fact-check) | ✅ déjà branché S3 |
| **compressor (réduit tokens)** | ✅ **S4 : branché + mesuré** |
| **router (choisit LLM)** | ✅ **S4 : branché + mesuré** |
| reformulator (rewrite canonique) | ⚠️ wired mais non utilisé dans le workflow — hors-scope S4 |
| filter (rank sources bge-m3) | ⚠️ wired mais non utilisé dans le workflow — hors-scope S4 |
| **`ChewingMetrics`** | ✅ **S4 : module nouveau** |
| **`StreamEvent::PreChewingMetrics`** | ✅ **S4 : variant nouveau** |

**Reformulator + filter** seront documentés comme **TODO Sprint S5+** dans
le commit S4 + un follow-up clair.

---

## 3. Livrables S4 (8 fichiers — 4 nouveaux + 4 patchés)

### 3.1 `crates/efe-coordinator/src/metrics.rs` (NOUVEAU, ~150 LOC)

Module nouveau exposant `ChewingMetrics` :

```rust
//! Module `metrics` — quantification du gain de pré-mâchage (Sprint S4).
//!
//! Capture les économies de tokens d'entrée du gros LLM grâce au pipeline
//! SLM (analyzer → router → compressor). Émis comme StreamEvent en fin de
//! mission et stocké dans MissionAResponse pour audit.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChewingMetrics {
    // ── Tokens (estimés via heuristique 1 token ≈ 4 chars) ───────────
    pub input_tokens_raw: u32,
    pub input_tokens_compressed: u32,
    pub llm_tokens_generated: u32,

    // ── Ratios (savings = 1 - compression_ratio) ──────────────────────
    pub compression_ratio: f32,   // [0.0, 1.0] : 1.0 = pas compressé
    pub savings_tokens: u32,
    pub savings_ratio: f32,       // [0.0, 1.0] : 0.6 = 60% économies

    // ── Latences par phase ────────────────────────────────────────────
    pub analyzer_latency_ms: u64,
    pub router_latency_ms: u64,
    pub compressor_latency_ms: u64,
    pub llm_latency_ms: u64,
    pub verifier_latency_ms: u64,
    pub total_slm_latency_ms: u64,
    pub total_pipeline_latency_ms: u64,

    // ── Choix Router + traçabilité ─────────────────────────────────────
    pub selected_llm: String,
    pub router_reason: String,
    pub router_estimated_cost_cents: u32,
}

impl ChewingMetrics {
    pub fn estimate_tokens(text: &str) -> u32 {
        ((text.chars().count() as f32 / 4.0).ceil()) as u32
    }

    /// Recalcule savings_* à partir de input_tokens_raw / _compressed.
    /// À appeler après avoir setté ces 2 champs.
    pub fn recompute_savings(&mut self) {
        self.compression_ratio = if self.input_tokens_raw > 0 {
            self.input_tokens_compressed as f32 / self.input_tokens_raw as f32
        } else { 1.0 };
        self.savings_tokens = self.input_tokens_raw
            .saturating_sub(self.input_tokens_compressed);
        self.savings_ratio = 1.0 - self.compression_ratio;
    }
}
```

### 3.2 `crates/efe-coordinator/src/events/stream.rs` (PATCH +15 LOC)

Ajout du variant :

```rust
PreChewingMetrics {
    swarm_id: Uuid,
    metrics: ChewingMetrics,
    ts: DateTime<Utc>,
},

// Dans tag() :
Self::PreChewingMetrics { .. } => "PRE_CHEWING_METRICS",
```

### 3.3 `crates/efe-coordinator/src/request.rs` (PATCH +5 LOC)

Ajout champ optionnel à `MissionAResponse` :

```rust
pub struct MissionAResponse {
    pub final_text: String,
    pub llm_used: String,
    pub cost_cents: u32,
    pub latency_ms: u64,
    pub verdicts: Vec<FactCheck>,
    pub audit_chain_hash: String,
    /// S4 — métriques de pré-mâchage (None si non calculées).
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,
}
```

### 3.4 `crates/efe-coordinator/src/lib.rs` (PATCH +2 LOC)

```rust
pub mod metrics;
pub use metrics::ChewingMetrics;
```

### 3.5 `crates/efe-coordinator/src/workflows/mission_a.rs` (PATCH ~30 LOC)

Instrumenter les latences + émettre l'event final + remplir
`MissionAResponse.chewing_metrics`. Voir §2.1 pour le pseudo-code.

### 3.6 `crates/efe-coordinator/tests/fixtures/missions/03_factual_compressible.json` (NOUVEAU)

Query LONGUE (≥ 500 chars) pour déclencher le compressor :

```json
{
  "name": "factual_compressible",
  "task_class": "FactualQa",
  "query": "Je suis en train de préparer une présentation sur la géographie politique de l'Europe occidentale, et il me faut absolument une réponse précise et concise. Pourrais-tu me confirmer quelle est la capitale officielle et administrative de la France, où sont concentrées les institutions politiques nationales comme le palais de l'Élysée, l'Assemblée nationale et le Sénat ? Réponds en une seule phrase courte.",
  "expected_keyword": "paris",
  "expected_lang": "fr",
  "candidate_sources": [
    {
      "id": "wp_paris_fr",
      "text": "Paris est la capitale et la plus grande ville de la France. Elle compte environ 2,1 millions d'habitants intra-muros."
    },
    {
      "id": "wp_france_admin",
      "text": "La République française a pour capitale Paris depuis le Xᵉ siècle, située sur la Seine en région Île-de-France."
    }
  ],
  "expected_verifier_verdict": "SUPPORTED",
  "expected_verifier_similarity_min": 0.3,
  "expected_compression_ratio_max": 0.7,
  "expected_savings_tokens_min": 30,
  "comment": "Fixture R2 Sprint S4 — query de ~440 chars qui DÉCLENCHE le compressor (seuil 500 chars dans mission_a.rs:215). Compression ratio attendu ≤ 0.7 (au moins 30% de réduction). Le LLM reçoit le summary compressé au lieu du paragraphe long."
}
```

Note : ~440 caractères → on est tout proche du seuil 500. Si on est sous 500
le compressor est skippé. Il faut tester ça avec une vraie mesure et
potentiellement ajuster le texte si on tombe en-dessous.

### 3.7 `crates/efe-coordinator/tests/live_missions.rs` (PATCH +180 LOC)

Ajout :
- Helper `make_coord_full_pipeline(provider)` — Coordinator avec
  analyzer + router + compressor + verifier (4 robots actifs)
- Helper `build_factual_request_for_full_pipeline(query, llm_id, sources)`
- Nouveau test `live_mission_a_full_pre_chewing_pipeline` (~120 LOC, 20+ assertions)

### 3.8 Doc architecture (CE FICHIER)

Référence durable du contrat S4.

---

## 4. Spec détaillée du test `live_mission_a_full_pre_chewing_pipeline`

### 4.1 Pré-conditions

| Pré-condition | Vérification | Action si fail |
|---|---|---|
| `EFE_LIVE_LLM=1` | env | `[skip]` |
| `EFE_LIVE_SLM=1` | env | `[skip]` |
| `ANTHROPIC_API_KEY` set | `ApiKey::from_env` | panic si EFE_LIVE_LLM |
| analyzer :8090 répond /health | curl 2s | `[skip]` |
| compressor :8092 répond /health | curl 2s | `[skip]` |
| verifier :8093 répond /health | curl 2s | `[skip]` |

### 4.2 Comportement nominal

```text
1. Charge la fixture 03_factual_compressible.json
2. Vérifie que fixture.query.len() ≥ 500 (sinon AssertionError)
3. Instancie :
   - ClaudeProvider::from_env()
   - AnalyzerClient::from_assets(:8090)
   - CompressorClient::from_assets(:8092)
   - VerifierClient::from_assets(:8093)
   - RouterClient::new()
4. ProviderRegistry + Claude
5. CoordinatorClients { analyzer, router, compressor, verifier : Some ;
                        reformulator, filter : None }
6. Coordinator::builder().registry(reg).clients(clients).build()?
7. Subscribe bus
8. Build MissionARequest avec sources
9. run_mission_a(&coord, req).await?
10. Drain events (timeout 300 ms, plus large car + d'events)
11. Vérifie présence event PRE_CHEWING_METRICS dans les tags
12. Extract chewing_metrics depuis MissionAResponse.chewing_metrics
13. eprintln! résumé enrichi
14. Assertions C1-C20 (cf. §4.3)
```

### 4.3 Assertions C1-C20

Numérotation `C*` (nouvelle famille pour S4) :

| # | Assertion | Justification |
|---|---|---|
| C1-C9 | (= B1-B9 de S3 héritées) | final_text, paris, llm_used, cost, latency, audit hash, wall-clock |
| C10-C12 | (= B10-B12 héritées S3) | verdicts non vide, verdict SUPPORTED, similarity ≥ 0.3 |
| C13-C15 | (= B13-B15 héritées S3) | events ROBOT_STARTED, PHEROMONE_DEPOSITED, VERDICT_READY |
| **C16** | `resp.chewing_metrics.is_some()` | MissionAResponse a bien le champ rempli |
| **C17** | `metrics.input_tokens_raw >= 100` | Fixture longue ≥ 500 chars ≈ 125 tokens |
| **C18** | `metrics.input_tokens_compressed < metrics.input_tokens_raw` | Le compressor a réduit |
| **C19** | `metrics.compression_ratio <= 0.7` | Au moins 30 % de savings (cf. fixture) |
| **C20** | `metrics.savings_tokens >= 30` | (cf. fixture expected_savings_tokens_min) |
| **C21** | `metrics.compressor_latency_ms > 0 && < 10_000` | Compressor a tourné dans un délai raisonnable |
| **C22** | `metrics.router_latency_ms < 50` | Heuristique in-process — sub-50 ms |
| **C23** | `metrics.selected_llm == claude_id` | Avec 1 seul LLM disponible, router pick lui |
| **C24** | `!metrics.router_reason.is_empty()` | Router a écrit une justification |
| **C25** | `tags.contains("PRE_CHEWING_METRICS")` | Event SSE émis avant SWARM_FINISHED |
| **C26** | `metrics.total_slm_latency_ms < metrics.total_pipeline_latency_ms` | SLM est une partie du pipeline |
| **C27** | `metrics.llm_latency_ms > 100` | Plancher Claude HTTPS |
| **C28** | `latency_ms < 25_000` | Cap S4 élargi (+ compressor vs S3) |

### 4.4 Sorties console attendues (cas nominal)

```text
=== Mission A FULL pre-chewing pipeline ===
  query_raw      : "Je suis en train de préparer une présentation sur…" (440 chars, ~110 tokens)
  llm_id         : claude_claude-sonnet-4-6
  4 robots actifs: analyzer + router + compressor + verifier

  ── Phase 1 : Pre-chewing SLM ──
  analyzer    : 5_234 ms   (20 entités extraites)
  router      :     2 ms   → Claude (reason: "highest quality_score")
  compressor  : 3_812 ms   (110 → 28 tokens, ratio=0.25, savings=82 tokens 75%)

  ── Phase 2 : LLM appelé sur prompt compressé ──
  llm         : 1_543 ms   (28 in tokens, ~12 out tokens)
  cost_cents  : 1
  final_text  : "Paris est la capitale de la France."

  ── Phase 3 : Verifier post-LLM ──
  verifier    : 3_421 ms
  verdict     : SUPPORTED
  similarity  : 0.85

  ── ChewingMetrics ──
  input_tokens_raw      : 110
  input_tokens_compressed : 28
  compression_ratio     : 0.25  (75% savings !)
  savings_tokens        : 82
  total_slm_latency_ms  : 12 467
  total_pipeline_latency_ms : 14 012
  audit_hash  : abc…def

  events_seen (~35) : [SWARM_STARTED, ROBOT_STARTED ×3, PHEROMONE_DEPOSITED ×20,
                        ROBOT_FINISHED ×3, LLM_*, VERDICT_READY,
                        PRE_CHEWING_METRICS, SWARM_FINISHED]

test live_mission_a_full_pre_chewing_pipeline ... ok
test result: ok. 3 passed; 0 failed   (S2 + S3 + S4)
```

---

## 5. Code Rust — squelettes prêts à coder

### 5.1 `src/metrics.rs` (complet, ~150 LOC)

Voir §3.1 ci-dessus pour la version exhaustive.

### 5.2 Patch `src/events/stream.rs`

```rust
// Ajouter aux imports :
use crate::metrics::ChewingMetrics;

// Ajouter au enum StreamEvent :
PreChewingMetrics {
    swarm_id: Uuid,
    metrics: ChewingMetrics,
    ts: DateTime<Utc>,
},

// Ajouter à impl StreamEvent::tag() :
Self::PreChewingMetrics { .. } => "PRE_CHEWING_METRICS",
```

### 5.3 Patch `src/workflows/mission_a.rs` (~30 LOC à ajouter)

À insérer juste avant le `return Ok(MissionAResponse { ... })` final :

```rust
// ── S4 : ChewingMetrics ──────────────────────────────────────────────
let mut chewing_metrics = ChewingMetrics::default();
chewing_metrics.input_tokens_raw = ChewingMetrics::estimate_tokens(&req.user_text);
chewing_metrics.input_tokens_compressed = ChewingMetrics::estimate_tokens(&final_prompt);
chewing_metrics.recompute_savings();
chewing_metrics.llm_tokens_generated = llm_resp.tokens_generated;
chewing_metrics.analyzer_latency_ms = analyzer_latency_ms_observed;  // tracker en cours
chewing_metrics.compressor_latency_ms = compressor_latency_ms_observed;
chewing_metrics.router_latency_ms = router_latency_ms_observed;
chewing_metrics.llm_latency_ms = llm_latency_ms_observed;
chewing_metrics.verifier_latency_ms = verifier_latency_ms_observed;
chewing_metrics.total_slm_latency_ms = chewing_metrics.analyzer_latency_ms
    + chewing_metrics.compressor_latency_ms
    + chewing_metrics.verifier_latency_ms;
chewing_metrics.total_pipeline_latency_ms = start.elapsed().as_millis() as u64;
chewing_metrics.selected_llm = selected_llm.clone();
chewing_metrics.router_reason = router_reason_observed;
chewing_metrics.router_estimated_cost_cents = router_estimated_cost;

// Émission de l'event PRE_CHEWING_METRICS
coord.bus.publish(StreamEvent::PreChewingMetrics {
    swarm_id,
    metrics: chewing_metrics.clone(),
    ts: Utc::now(),
}).await;

return Ok(MissionAResponse {
    final_text,
    llm_used: selected_llm,
    cost_cents,
    latency_ms: start.elapsed().as_millis() as u64,
    verdicts,
    audit_chain_hash: audit.finalize(),
    chewing_metrics: Some(chewing_metrics),   // S4 NEW
});
```

Pour les latences par robot, on doit **instrumenter** chaque appel
`invoke()` avec un `let t = Instant::now();` autour. On utilisera donc
~6 nouvelles variables locales dans `run_mission_a`.

### 5.4 Helpers + test S4

Voir §6 (squelette à compléter quand on code).

---

## 6. Plan d'implémentation chirurgical S4

10 sub-sprints de 20-40 min, committables indépendamment :

| # | Sprint | Durée | Livrable | Critère |
|---|---|---|---|---|
| **S4.1** | `src/metrics.rs` module nouveau | 30 min | `ChewingMetrics` + helpers | compile clean |
| **S4.2** | `src/events/stream.rs` patch | 20 min | variant `PreChewingMetrics` + `tag()` | compile clean, sérialisation roundtrip |
| **S4.3** | `src/request.rs` patch | 10 min | champ `chewing_metrics: Option<...>` | `#[serde(default)]`, 0 régression |
| **S4.4** | `src/lib.rs` re-exports | 5 min | `pub mod metrics` + `pub use ChewingMetrics` | compile clean |
| **S4.5** | `src/workflows/mission_a.rs` instrumentation | 60 min | tracking latences + émission event + remplir Response | les tests offline existants passent toujours |
| **S4.6** | Fixture `03_factual_compressible.json` | 10 min | query ≥ 500 chars | JSON valide + len ≥ 500 |
| **S4.7** | `tests/live_missions.rs` helpers + test | 45 min | `make_coord_full_pipeline` + `live_mission_a_full_pre_chewing_pipeline` (28 assertions) | skip propre |
| **S4.8** | Compile + clippy + non-régression | 15 min | 396 → 397 PASS, 0 warning S4 | 397/397 PASS |
| **S4.9** | Test live réel (action user) | 20 min | run + verify chewing_metrics observées | 28/28 PASS attendu |
| **S4.10** | `S4_LIVE_VALIDATION.md` + commit final | 30 min | scellement | commit `feat(phase-3-r2-s4): …` |

**Total temps** : ~4 h 15 min cumulés + buffer 1 h 45 = **6 heures**.

---

## 7. Critères d'acceptation binaires S4

S4 est **livré** quand TOUS ces critères sont vrais :

- [ ] `src/metrics.rs` existe avec `ChewingMetrics` struct + `estimate_tokens` + `recompute_savings`
- [ ] `ChewingMetrics` est re-exporté via `efe_coordinator::ChewingMetrics`
- [ ] `StreamEvent::PreChewingMetrics` ajouté, sérialise en `"type": "PRE_CHEWING_METRICS"`
- [ ] `StreamEvent::tag()` retourne `"PRE_CHEWING_METRICS"` pour le nouveau variant
- [ ] `MissionAResponse.chewing_metrics: Option<ChewingMetrics>` avec `#[serde(default)]`
- [ ] `run_mission_a` instrumente les latences par robot + émet `PreChewingMetrics` event + remplit `chewing_metrics` dans la response
- [ ] Fixture `03_factual_compressible.json` avec query ≥ 500 chars
- [ ] Test `live_mission_a_full_pre_chewing_pipeline` avec 28 assertions C1-C28
- [ ] Skip propre × 4 cas (EFE_LIVE_LLM / EFE_LIVE_SLM / SLM down / clé manquante)
- [ ] **0 régression** sur la suite offline : 396 → 397 PASS (le test S4 en skip mode compte +1)
- [ ] **0 warning clippy** sur le code S4 (metrics.rs + stream.rs patch + workflows/mission_a.rs patch + test)
- [ ] Test live PASS avec env + 3 daemons UP (analyzer, compressor, verifier — pas besoin de reformulator)
- [ ] 28/28 assertions PASS en live
- [ ] `compression_ratio ≤ 0.7` observé (≥ 30 % savings)
- [ ] `savings_tokens ≥ 30` observé
- [ ] Event `PRE_CHEWING_METRICS` capturé dans le bus
- [ ] `S4_LIVE_VALIDATION.md` scellé avec valeurs observées
- [ ] Commit final `feat(phase-3-r2-s4): full pre-chewing pipeline + ChewingMetrics — savings X% live-validated`

---

## 8. Hors-scope S4 (présents dans S5+)

| Item | Sprint cible | Pourquoi pas S4 |
|---|---|---|
| **Reformulator** intégré au workflow | S5b ou Phase 4 | Nécessite modification structurelle de `run_mission_a` (étape entre analyzer et router) |
| **Filter** intégré au workflow | S5b ou Phase 4 | idem, + besoin d'instancier `BgeM3Embedder` (feature `fastembed-real` → téléchargement HF) |
| Mission B (3-LLM vote) | S5 | Multi-provider |
| Router tuning Pareto par task_class | S6 | Table de scoring statique à calibrer |
| Mission Speculative | S7 | drafter/verifier |
| Failure modes | S8 | tests négatifs |
| 5+ fixtures variées | S9 | Savings report multi-cas |
| Tokenizer précis (tiktoken) | Phase 4 | Optimisation, l'heuristique 1/4 est suffisante pour S4 |

---

## 9. Risques connus & mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Fixture `query.len() < 500` → compressor skip → C18/C19 fail | Moyenne | C18-C20 fail | Vérifier la longueur dans la fixture S4.6 + dans le test (assert > 500 avant run) |
| Compressor produit un summary plus long que prévu (peu de compression) | Moyenne | C19 (ratio > 0.7) fail | Élargir le cap à 0.8 si nécessaire post-run |
| `router_latency_ms > 50 ms` (machine sous charge) | Faible | C22 fail | Cap conservatif (50 ms pour heuristique in-process Rust) |
| Modification de `mission_a.rs` casse les tests offline | Moyenne | regression | Tester en S4.8 OBLIGATOIREMENT avant S4.9 |
| `#[serde(default)]` sur Option<ChewingMetrics> ne suffit pas (vieux JSON casse) | Faible | sérialisation broken | Tester roundtrip dans `request.rs` tests inline |
| Event `PRE_CHEWING_METRICS` perdu dans le timeout 300 ms | Faible | C25 fail | Élargir timeout à 500 ms si besoin |

---

## 10. Vérification post-livraison

Une fois S4 commit, on doit pouvoir reproduire :

```bash
$ ./scripts/slm/start.sh && ./scripts/slm/health.sh    # 4/4 ● UP
$ ./scripts/slm/check_live_env.sh --required anthropic
$ EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 \
    cargo test -p efe-coordinator --features all-providers \
      --test live_missions -- --nocapture

running 3 tests
=== Mission A naked ===            (S2 — déjà validé)
test live_mission_a_naked_no_slm_just_claude ... ok

=== Mission A with analyzer + verifier ===   (S3 — déjà validé)
test live_mission_a_with_analyzer_verifier ... ok

=== Mission A FULL pre-chewing pipeline ===  (S4 — NOUVEAU)
  query_raw   : "Je suis en train de préparer…" (440 chars, ~110 tokens)
  …
  compressor  : 3 812 ms (110 → 28 tokens, ratio=0.25, savings=82 tokens 75%)
  …
  ── ChewingMetrics ──
  compression_ratio    : 0.25  (75% savings !)
  savings_tokens       : 82
  total_pipeline_latency_ms : 14 012
  …
test live_mission_a_full_pre_chewing_pipeline ... ok

test result: ok. 3 passed; 0 failed
```

**Si cette sortie est obtenue → S4 est scellé et le but premier de R2 est
prouvé en conditions réelles.**

---

## 11. Lien avec les sprints suivants

S4 produit l'infrastructure clé pour S5-S9 :

### Pour S5 (Mission B 3-LLM vote)
- `ChewingMetrics` réutilisable pour mesurer le **savings mutualisé** (1 compresseur × 3 LLMs)
- Pattern d'instrumentation `Instant::now()` × phase → généralisable

### Pour S6 (Router tuning)
- `metrics.router_reason` permet de débugger les choix Pareto
- `metrics.selected_llm` permet d'asserter les choix attendus (avec multi-LLM)

### Pour S9 (Chewing savings report)
- `ChewingMetrics` exporté en JSON via le bus + Response
- 5 fixtures × 2 modes (baseline vs full) → tableau comparatif

---

## 12. Conclusion

S4 est le sprint **le plus ambitieux** de R2 :
- **Premier** sprint qui modifie le code source (vs S2-S3 = test-only)
- **Premier** module nouveau (`metrics.rs`)
- **Premier** variant `StreamEvent` ajouté
- **Premier** chiffre dur du gain de pré-mâchage en conditions réelles

Si S4 atteint **75 % de savings sur tokens d'entrée**, c'est l'argument
économique défintif de R2 : **EFE divise par 3-4 le coût d'un gros LLM
sur des queries longues, en gardant l'audit chain + le fact-check.**

**Prochaine action** : démarrer S4.1 (créer `src/metrics.rs`).
