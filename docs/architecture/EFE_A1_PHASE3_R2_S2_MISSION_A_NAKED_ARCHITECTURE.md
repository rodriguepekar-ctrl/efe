# EFE Phase 3 R2 — Sprint S2 : Mission A Naked

**Précédence** : S1 validé live (`scripts/slm/S1_LIVE_VALIDATION.md`)
**Suivant** : S3 — Mission A + analyzer + verifier
**Statut** : Spec ready-to-implement
**Effort estimé** : 2 heures
**Coût $ estimé** : < 0.05 $ (5 calls Claude × 1 ¢ max)

---

## 0. Pourquoi ce sprint, dans cet ordre

> **Avant de prouver que le pré-mâchage fonctionne (S3-S9), on prouve que la
> tuyauterie Coordinator ↔ Provider fonctionne SANS pré-mâchage.**

S2 est volontairement réduit au minimum vital : un seul test, un seul provider
(Claude), zéro nano-robot, une seule fixture factuelle. Son rôle est de
**verrouiller l'invariant** : `MultiLlmBridge` → `LlmProvider::call()` →
`run_mission_a` → `MissionAResponse` fonctionne en conditions réelles.

Si S2 passe, alors **tous** les sprints suivants (S3-S9) ajoutent des couches
verticales (nano-robots, vote multi-LLM, métriques) en sachant que le socle
horizontal est sain. Si S2 échoue, on debug ce socle avant de toucher au reste.

> **C'est l'équivalent du "Hello World" pour Mission A — minimaliste mais
> fondateur.**

---

## 1. Audit chirurgical pré-S2 — état exact du codebase

Audit effectué le 2026-05-27 (cf. exploration Agent dans la session). Faits
concrets (chemins + signatures) :

### 1.1 `MissionARequest` — structure publique

**Fichier** : `crates/efe-coordinator/src/request.rs:20-44`

```rust
pub struct MissionARequest {
    pub user_text: String,
    pub task_class: TaskClass,
    pub available_llms: Vec<String>,
    pub llm_metadata: HashMap<String, LlmMetadata>,
    pub budget: Budget,
    pub candidate_sources: Vec<CandidateSourceRef>,
    pub max_total_latency: Duration,
}
```

**Pas de `::new()`, `::simple()` ni `Default::default()` publics.** Construction
champ-par-champ obligatoire (cf. test workflow_mission_a.rs:35).

### 1.2 `MissionAResponse` — structure de retour

```rust
pub struct MissionAResponse {
    pub final_text: String,         // texte généré par le LLM (et éventuellement filtré par verifier)
    pub llm_used: String,            // ID du provider qui a répondu
    pub cost_cents: u32,             // coût agrégé en cents entiers
    pub latency_ms: u64,             // latence totale Mission A (pas juste LLM)
    pub verdicts: Vec<FactCheck>,    // verdicts du verifier (vide si pas activé)
    pub audit_chain_hash: String,    // hash HMAC-SHA256 de la chaîne d'audit
}
```

### 1.3 `run_mission_a` — point d'entrée

**Fichier** : `crates/efe-coordinator/src/workflows/mission_a.rs:38`

```rust
pub async fn run_mission_a(
    coord: &Coordinator,
    req: MissionARequest,
) -> CoordResult<MissionAResponse>
```

C'est une **fonction libre**, pas une méthode `Coordinator::execute_mission_a`.
Retour direct (pas de stream — celui-ci passe par `coord.bus` séparément).

### 1.4 Sélection du LLM sans router

**Fichier** : `crates/efe-coordinator/src/workflows/mission_a.rs:131-200`

```rust
let llm_id = if let Some(router) = &coord.clients.router {
    router.invoke(...).await?.chosen_llm    // si router présent
} else {
    req.available_llms.first().cloned()      // sinon : 1er de la liste
        .ok_or_else(|| CoordError::Config("no available LLMs".into()))?
};
```

**Implication pour S2** : pour forcer Claude, il suffit de mettre
`available_llms = vec!["claude_claude-sonnet-4-6".into()]` en premier.

### 1.5 `CoordinatorBuilder` — minimaliste

Tous les setters sont optionnels :

```rust
let coord = Coordinator::builder()
    .registry(reg)        // optionnel mais évidemment requis pour avoir un LLM
    .build()?;
```

Defaults appliqués :
- `clients` → `CoordinatorClients::empty()` (tous les 6 champs `None`)
- `config` → `CoordinatorConfig::default()` (deadline 30s, max_concurrency 8, tenant "default")
- `bus` → `Arc::new(InMemoryBus::new())`
- `vote` → `VoteEngine::default()`

**Pour S2** : `Coordinator::builder().registry(reg).build()?` suffit. Aucun
appel `.clients()` requis.

### 1.6 `ClaudeProvider::from_env()` — pattern R1a/R1b déjà validé live

**Fichier** : `crates/efe-coordinator/src/llm/providers/anthropic.rs:69-71`

```rust
pub fn from_env() -> CoordResult<Self> {
    let key = ApiKey::from_env("ANTHROPIC_API_KEY", "sk-ant-")?;
    Ok(Self::new(key, DEFAULT_CLAUDE_MODEL))
}
```

`DEFAULT_CLAUDE_MODEL = "claude-sonnet-4-6"` (cf. `providers/mod.rs:67`).
`provider.id()` retourne `"claude_claude-sonnet-4-6"`.

### 1.7 Pattern de test live env-gated

**Fichier référence** : `crates/efe-coordinator/tests/live_providers.rs:29-39`

```rust
fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_LLM").as_deref() == Ok("1")
}

#[tokio::test]
async fn live_xxx() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY to enable");
        return;
    }
    // ...
}
```

### 1.8 Déclaration test binary dans `Cargo.toml`

**Pattern existant à dupliquer** :

```toml
[[test]]
name = "live_providers"
path = "tests/live_providers.rs"
required-features = ["all-providers"]
```

### 1.9 Tests existants qui valident `run_mission_a` (offline)

**Fichier** : `crates/efe-coordinator/tests/workflow_mission_a.rs`

- `mission_a_runs_with_mock_llm_only` (ligne ~28)
- `mission_a_publishes_swarm_started_and_finished` (ligne ~56)

Tous utilisent `MockLlmProvider`. **Aucun test ne combine Coordinator empty +
vrais providers + assertion sémantique factuelle**. C'est précisément le trou
que S2 vient combler.

### 1.10 Verdict audit

**Excellente nouvelle** : 100 % du code Rust requis pour S2 existe déjà.
S2 est uniquement un test à écrire (≈80 LOC) + 1 fixture JSON (≈10 lignes) +
1 entrée Cargo.toml (4 lignes). **Pas une seule ligne de code source à
modifier.** Si le test échoue, c'est un vrai bug du workflow ou du provider —
pas un défaut d'instrumentation.

---

## 2. Architecture S2 — Vue d'ensemble

### 2.1 Flow Mission A naked (sans nano-robots)

```text
   ┌──────────────────────────────────────────────────────────────────────┐
   │  TEST live_mission_a_naked_no_slm_just_claude                        │
   │  EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY                                   │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  ProviderRegistry                                                    │
   │  ├─ "claude_claude-sonnet-4-6" → Arc<ClaudeProvider>                 │
   │                                                                       │
   │  Coordinator::builder().registry(reg).build()?                       │
   │  ├─ clients : CoordinatorClients::empty()  (TOUS None)               │
   │  ├─ config  : default (deadline 30s)                                 │
   │  ├─ bus     : InMemoryBus                                            │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionARequest {                                                   │
   │    user_text: "Quelle est la capitale de la France ? …",            │
   │    task_class: FactualQa,                                            │
   │    available_llms: vec!["claude_claude-sonnet-4-6"],                │
   │    llm_metadata: { claude: LlmMetadata { ... } },                    │
   │    budget: { max_cost_cents: 10, max_latency_ms: 30_000, ... },     │
   │    candidate_sources: vec![],                                        │
   │    max_total_latency: Duration::from_secs(30),                       │
   │  }                                                                    │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  run_mission_a(&coord, req).await?                                   │
   │                                                                       │
   │  → analyzer ?  → None  → SKIP                                        │
   │  → filter/router/reformulator ? → None → SKIP                        │
   │  → compressor ? → None → SKIP                                        │
   │  → LLM choisi = available_llms.first() = "claude_claude-sonnet-4-6"  │
   │  → Bridge.call(claude, llm_call).await?                              │
   │     ├─ HTTPS POST api.anthropic.com/v1/messages                      │
   │     ├─ ApiKey injecté (Authorization: x-api-key)                     │
   │     ├─ Retry exponential + circuit breaker                           │
   │     └─ Response : { content: "Paris est ...", usage: { ... } }       │
   │  → verifier ? → None → SKIP                                          │
   │  → final_text = response.text                                        │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionAResponse {                                                  │
   │    final_text: "Paris est la capitale de la France.",                │
   │    llm_used: "claude_claude-sonnet-4-6",                             │
   │    cost_cents: 1-2,                                                  │
   │    latency_ms: 800-2000,                                             │
   │    verdicts: vec![],   // verifier non activé                        │
   │    audit_chain_hash: "abc123...64chars",                             │
   │  }                                                                    │
   └──────────────────────────────────────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  ASSERTIONS                                                          │
   │  assert!(resp.final_text.to_lowercase().contains("paris"))           │
   │  assert_eq!(resp.llm_used, "claude_claude-sonnet-4-6")               │
   │  assert!(resp.cost_cents <= 5)                                       │
   │  assert!(resp.latency_ms < 30_000)                                   │
   │  assert_eq!(resp.audit_chain_hash.len(), 64)                         │
   │  assert!(resp.verdicts.is_empty())   // pas de verifier              │
   └──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Ce que S2 ne teste PAS (volontairement)

| Composant | Statut S2 | Sprint qui le couvre |
|---|---|---|
| Analyzer | non | S3 |
| Filter | non | S4 |
| Router (nano-robot) | non | S4 + S6 |
| Reformulator | non | S4 |
| Compressor | non | S4 |
| Verifier | non | S3 |
| Multi-LLM vote | non | S5 |
| Speculative drafting | non | S7 |
| Failure modes | non | S8 |
| Métriques pré-mâchage | non | S9 |

S2 = **isolation maximale du wiring** Coordinator ↔ Provider. Toutes les
features avancées arrivent dans les sprints suivants.

---

## 3. Livrables S2 (4 fichiers — 0 ligne de code source modifiée)

### 3.1 `crates/efe-coordinator/tests/live_missions.rs` (NOUVEAU)

Le fichier de test principal. Contient :
- 1 helper `live_enabled()` (copie de `live_providers.rs`)
- 1 helper `load_fixture(name)` qui lit `tests/fixtures/missions/*.json`
- 1 helper `build_mission_a_request(query, task_class, llm_id)` qui construit
  un `MissionARequest` minimal valide à partir des params
- **1 test** : `live_mission_a_naked_no_slm_just_claude`
- (préparé pour S3+) emplacement pour les futurs tests `live_mission_a_with_*`

### 3.2 `crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json` (NOUVEAU)

```json
{
  "name": "factual_simple",
  "task_class": "FactualQa",
  "query": "Quelle est la capitale de la France ? Réponds en une seule phrase courte.",
  "expected_keyword": "paris",
  "expected_lang": "fr"
}
```

Format conçu pour être réutilisé en S3-S9 (les fixtures vont s'enrichir).

### 3.3 `crates/efe-coordinator/Cargo.toml` (PATCH 1 entrée)

```toml
[[test]]
name = "live_missions"
path = "tests/live_missions.rs"
required-features = ["all-providers"]
```

### 3.4 `docs/architecture/EFE_A1_PHASE3_R2_S2_MISSION_A_NAKED_ARCHITECTURE.md` (CE FICHIER)

Le présent document. Reste après livraison comme référence durable du
contrat S2.

---

## 4. Spec détaillée du test `live_mission_a_naked_no_slm_just_claude`

### 4.1 Pré-conditions

| Pré-condition | Vérification | Action si fail |
|---|---|---|
| `EFE_LIVE_LLM=1` | `std::env::var("EFE_LIVE_LLM")` | `eprintln!("[skip] …"); return;` (PASS) |
| `ANTHROPIC_API_KEY` set | `ApiKey::from_env("ANTHROPIC_API_KEY", "sk-ant-")` | `expect()` → panic test si EFE_LIVE_LLM=1 mais clé manquante |
| Feature `all-providers` active | Compile time via `#![cfg(feature = "all-providers")]` | Test pas compilé sinon |

### 4.2 Comportement nominal

```text
1. Charge la fixture 01_factual_simple.json
2. Instancie ClaudeProvider::from_env()? → Arc<ClaudeProvider>
3. ProviderRegistry::new() + .register(arc)
4. Coordinator::builder().registry(reg).build()?
5. Construit MissionARequest avec :
   - user_text = fixture.query
   - task_class = TaskClass::FactualQa
   - available_llms = vec![claude_id]   // l'ID retourné par provider.id()
   - llm_metadata = { claude_id: LlmMetadata::default_for_claude() }
   - budget = Budget { max_cost_cents: 10, max_latency_ms: 30_000, min_quality_score: 0.0 }
   - candidate_sources = vec![]   // pas de source = verifier auto-skip
   - max_total_latency = Duration::from_secs(30)
6. Mesure t0 = Instant::now()
7. resp = run_mission_a(&coord, req).await.expect("mission_a OK")
8. Mesure latence wall-clock = t0.elapsed()
9. eprintln! du résumé (final_text, llm_used, cost, latency)
10. ASSERTIONS (cf. §4.3)
```

### 4.3 Assertions

| # | Assertion | Justification |
|---|---|---|
| A1 | `!resp.final_text.is_empty()` | LLM doit avoir produit du texte |
| A2 | `resp.final_text.to_lowercase().contains("paris")` | Consensus factuel |
| A3 | `resp.llm_used == "claude_claude-sonnet-4-6"` | C'est bien Claude qui a répondu (pas un mock) |
| A4 | `resp.cost_cents <= 5` | Cap budget — Sonnet sur 50 tokens = ~1 ¢ |
| A5 | `resp.latency_ms < 30_000` | Cap latence (deadline budget) |
| A6 | `resp.latency_ms >= 100` | Si < 100 ms = suspect (mock ?) |
| A7 | `resp.audit_chain_hash.len() == 64` | HMAC-SHA256 hex |
| A8 | `resp.verdicts.is_empty()` | Pas de verifier nano-robot → pas de FactCheck |
| A9 | wall_clock_elapsed < 35_000 ms | Sanity check global (incluant overhead test) |

### 4.4 Comportement en cas d'absence des variables

```bash
# Sans EFE_LIVE_LLM :
$ cargo test -p efe-coordinator --features all-providers --test live_missions
running 1 test
test live_mission_a_naked_no_slm_just_claude ... ok   # skip propre
test result: ok. 1 passed; 0 failed
```

```bash
# Avec EFE_LIVE_LLM=1 mais clé manquante :
$ EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers --test live_missions
running 1 test
test live_mission_a_naked_no_slm_just_claude ... FAILED
  panicked at 'ANTHROPIC_API_KEY must be set: Config("env var ANTHROPIC_API_KEY not set")'
```

```bash
# Cas nominal :
$ EFE_LIVE_LLM=1 ANTHROPIC_API_KEY=sk-ant-... cargo test ...
running 1 test
test live_mission_a_naked_no_slm_just_claude ... === Mission A naked ===
  query        : Quelle est la capitale de la France ? …
  llm_used     : claude_claude-sonnet-4-6
  cost_cents   : 1
  latency_ms   : 1245
  audit_hash   : a3f8...  (64 chars)
  final_text   : "Paris est la capitale de la France."
ok

test result: ok. 1 passed; 0 failed
```

---

## 5. Code Rust complet du test (réservé pour l'implémentation)

```rust
// SPDX-License-Identifier: UNLICENSED
//! Tests live Phase 3 R2 — Missions Réelles Live.
//!
//! Sprint S2 : Mission A naked (Coordinator + Claude, aucun nano-robot).
//! Voir : docs/architecture/EFE_A1_PHASE3_R2_S2_MISSION_A_NAKED_ARCHITECTURE.md
//!
//! ## Activation
//! ```bash
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
//!   --test live_missions -- --nocapture
//! ```

#![cfg(feature = "all-providers")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::llm::ProviderRegistry;
use efe_coordinator::{
    run_mission_a, ClaudeProvider, Coordinator, MissionARequest,
};
use efe_coordinator::request::{Budget, LlmMetadata, TaskClass};

// ─── Helpers ────────────────────────────────────────────────────────────────

fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_LLM").as_deref() == Ok("1")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/missions")
        .join(name)
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let s = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&s).expect("valid JSON")
}

/// Construit un MissionARequest factuel minimal forçant un LLM précis.
fn build_factual_request(query: &str, llm_id: &str) -> MissionARequest {
    let mut metadata = HashMap::new();
    metadata.insert(
        llm_id.to_string(),
        LlmMetadata {
            // Valeurs raisonnables pour un small factual question.
            cost_per_million_input_cents: 300,
            cost_per_million_output_cents: 1500,
            quality_score: 0.92,
            latency_p50_ms: 1200,
        },
    );
    MissionARequest {
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            max_cost_cents: 10,
            max_latency_ms: 30_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(30),
    }
}

// ─── Sprint S2 — Mission A naked ────────────────────────────────────────────

#[tokio::test]
async fn live_mission_a_naked_no_slm_just_claude() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY to enable");
        return;
    }

    // 1. Charge la fixture.
    let fixture = load_fixture("01_factual_simple.json");
    let query = fixture["query"].as_str().expect("fixture.query");
    let expected_keyword = fixture["expected_keyword"]
        .as_str()
        .expect("fixture.expected_keyword");

    // 2. Provider Claude réel (clé via env).
    let claude = Arc::new(
        ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY must be set"),
    );
    let claude_id = claude.id();
    eprintln!("=== Mission A naked ===");
    eprintln!("  query    : {query}");
    eprintln!("  llm_id   : {claude_id}");

    // 3. Registry avec 1 seul provider, 0 nano-robot.
    let reg = ProviderRegistry::new();
    reg.register(claude);

    // 4. Coordinator minimal (clients par défaut = tous None).
    let coord = Coordinator::builder()
        .registry(reg)
        .build()
        .expect("coordinator builds");

    // 5. Construit la requête.
    let req = build_factual_request(query, &claude_id);

    // 6. Exécute Mission A et mesure le wall-clock.
    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req).await.expect("mission_a ok");
    let wall_ms = started.elapsed().as_millis() as u64;

    eprintln!("  cost_cents : {}", resp.cost_cents);
    eprintln!("  latency_ms : {} (mission_a internal) / {} (wall-clock)",
        resp.latency_ms, wall_ms);
    eprintln!("  audit_hash : {}…{}",
        &resp.audit_chain_hash[..8],
        &resp.audit_chain_hash[resp.audit_chain_hash.len() - 8..]);
    eprintln!("  final_text : {:?}", resp.final_text);

    // 7. Assertions (cf. §4.3 de l'architecture).
    assert!(!resp.final_text.is_empty(),                       // A1
        "final_text non vide");
    assert!(resp.final_text.to_lowercase().contains(expected_keyword),  // A2
        "réponse doit contenir '{expected_keyword}', got: {:?}", resp.final_text);
    assert_eq!(resp.llm_used, claude_id,                        // A3
        "claude doit être le LLM utilisé");
    assert!(resp.cost_cents <= 5,                               // A4
        "coût ≤ 5 ¢ pour une question simple, got {} ¢", resp.cost_cents);
    assert!(resp.latency_ms < 30_000,                           // A5
        "latence < 30 s, got {} ms", resp.latency_ms);
    assert!(resp.latency_ms >= 100,                             // A6
        "latence ≥ 100 ms (sinon mock suspect), got {} ms", resp.latency_ms);
    assert_eq!(resp.audit_chain_hash.len(), 64,                 // A7
        "audit_chain_hash = HMAC-SHA256 hex (64 chars)");
    assert!(resp.verdicts.is_empty(),                           // A8
        "pas de verifier → verdicts vide");
    assert!(wall_ms < 35_000,                                   // A9
        "wall-clock < 35 s (sanity), got {wall_ms} ms");
}
```

**Total : ~95 LOC test + helpers.** Aucune dépendance interne supplémentaire.

---

## 6. Fixtures `tests/fixtures/missions/`

### 6.1 Convention du format

```json
{
  "name": "<kebab-or-snake-case>",
  "task_class": "FactualQa|CodeGen|SummaryShort|LongReasoning|...",
  "query": "<le texte exact envoyé à Mission A>",
  "expected_keyword": "<token attendu dans le final_text, lowercase>",
  "expected_lang": "fr|en"
}
```

Format extensible : les futurs sprints ajouteront des champs comme
`min_compression_ratio`, `min_verifier_confidence`, `expected_route` etc.

### 6.2 Fixture S2 — `01_factual_simple.json`

```json
{
  "name": "factual_simple",
  "task_class": "FactualQa",
  "query": "Quelle est la capitale de la France ? Réponds en une seule phrase courte.",
  "expected_keyword": "paris",
  "expected_lang": "fr"
}
```

**Pourquoi cette query précisément** :
- **Factuelle vérifiable** : "paris" est l'unique réponse correcte.
- **Robuste à la langue** : `to_lowercase()` matche "Paris" / "PARIS".
- **Court** : ~25 tokens d'entrée → coût garanti < 1 ¢ sur Sonnet.
- **Français** : prouve que Claude ne se limite pas à l'anglais.
- **"Réponds en une seule phrase courte"** : empêche les digressions qui
  feraient sortir Paris du final_text (Claude est verbeux par défaut).

---

## 7. Cargo.toml patch

**Ajout dans `crates/efe-coordinator/Cargo.toml`** après l'entrée
`live_providers` (lignes 132-135 actuelles) :

```toml
[[test]]
name = "live_missions"
path = "tests/live_missions.rs"
required-features = ["all-providers"]
```

**Aucune dépendance supplémentaire** (toutes déjà présentes via `dev-dependencies`).

---

## 8. Plan d'implémentation chirurgical S2

Découpé en 6 sub-sprints de 15-25 minutes chacun, committables indépendamment :

### S2.1 — Création de la structure de fichiers (5 min)

- `mkdir -p crates/efe-coordinator/tests/fixtures/missions`
- `touch crates/efe-coordinator/tests/live_missions.rs`
- `touch crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json`
- Patch Cargo.toml (entrée `[[test]] live_missions`)

**Critère** : `cargo build -p efe-coordinator --features all-providers --tests`
compile sans erreur (test vide).

### S2.2 — Helpers de base (15 min)

Implémenter dans `live_missions.rs` :
- `live_enabled()`
- `fixture_path()`
- `load_fixture()`
- `build_factual_request()`

**Critère** : `cargo check -p efe-coordinator --features all-providers --tests`
sans warning. (Test pas encore écrit donc pas d'erreur lint sur dead-code grâce
au `#![cfg(feature = "all-providers")]`.)

### S2.3 — Écriture du test minimal (15 min)

Ajouter le test `live_mission_a_naked_no_slm_just_claude` SANS les assertions
A4-A9 (juste A1, A2, A3 dans un premier temps).

**Critère** : `cargo test -p efe-coordinator --features all-providers --test live_missions`
PASS en mode skip (sans `EFE_LIVE_LLM=1`).

### S2.4 — Écriture de la fixture JSON (5 min)

Le fichier `01_factual_simple.json` avec le contenu de §6.2.

**Critère** : `python3 -m json.tool < ...01_factual_simple.json` valide.

### S2.5 — Test live réel (20 min)

```bash
export ANTHROPIC_API_KEY='sk-ant-...'
EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
  --test live_missions -- --nocapture
```

**Critère** : test PASS avec `final_text` contenant "paris". Affichage console
du résumé conforme à §4.4.

### S2.6 — Hardening : ajout assertions A4-A9 (10 min)

Une fois A1-A3 vertes, ajouter les assertions de cap (coût, latence min/max,
hash size, verdicts vide, wall-clock). Re-tester en live.

**Critère** : 9/9 assertions PASS.

### S2.7 — Commit final (5 min)

```bash
git add tests/live_missions.rs tests/fixtures/missions/ Cargo.toml docs/architecture/...
git commit -m "feat(phase-3-r2-s2): Mission A naked live-validated (Coordinator + Claude)"
```

**Total temps** : ~75 min cumulés + buffer 30 min imprévus = **2 heures**.

---

## 9. Critères d'acceptation binaires S2

S2 est **livré** quand TOUS ces critères sont vrais :

- [ ] `crates/efe-coordinator/tests/live_missions.rs` existe et contient le test
- [ ] `crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json` existe
- [ ] `crates/efe-coordinator/Cargo.toml` déclare le test binary `live_missions` avec `required-features = ["all-providers"]`
- [ ] `cargo build -p efe-coordinator --features all-providers --tests` → 0 erreur, 0 warning sur le code S2
- [ ] `cargo test -p efe-coordinator --features all-providers --test live_missions` (sans `EFE_LIVE_LLM`) → `1 passed; 0 failed` (skip propre)
- [ ] `EFE_LIVE_LLM=1 ANTHROPIC_API_KEY=sk-ant-... cargo test … --test live_missions -- --nocapture` → **`1 passed; 0 failed`** avec `final_text` contenant "paris"
- [ ] 9/9 assertions A1-A9 PASS
- [ ] Coût observé ≤ 2 ¢
- [ ] Latence observée < 5 000 ms (p99 typique sur Sonnet < 3 000 ms)
- [ ] **0 régression** sur la suite offline : `cargo test -p efe-coordinator --features all-providers --tests` (tous binaires) → 412+ PASS comme avant R2
- [ ] Commit final pushe avec message au format `feat(phase-3-r2-s2): …`

---

## 10. Hors-scope S2 (donc PRÉSENTS dans S3-S10)

| Item | Sprint cible | Pourquoi pas S2 |
|---|---|---|
| Analyzer nano-robot | S3 | Ajoute complexité (parse intent), 1 sprint dédié |
| Verifier nano-robot | S3 | Ditto (parse FactCheck verdicts) |
| Compressor + métriques | S4 | Nécessite `metrics::ChewingMetrics` (nouveau module) |
| Router intelligent | S6 | Stratégie Pareto à tuner = sprint propre |
| Mission B (3-LLM vote) | S5 | Multi-provider |
| Mission Speculative | S7 | Pattern drafter/verifier |
| Failure modes (429, SLM down) | S8 | Tests négatifs |
| Rapport savings tokens | S9 | Comparaison baseline vs full |
| `R2_LIVE_VALIDATION.md` final | S10 | Scellement global |

**Anti-pattern à éviter en S2** : si on commence à ajouter "juste l'analyzer
parce que c'est facile", on perd le bénéfice d'isolation. **S2 reste STRICTEMENT
naked.** Toute extension va en S3.

---

## 11. Risques connus & mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Claude répond verbeusement → "Paris" pas dans le final_text | Faible | A2 fail | Query inclut "Réponds en une seule phrase courte" |
| Latence Claude > 30 s (problème réseau, modèle saturé) | Très faible | A5 fail | Budget `max_latency_ms: 30_000` = 30 s — généreux |
| Quota Claude épuisé (rare) | Faible | erreur 429 | Retry policy déjà active dans ClaudeProvider R1a |
| `claude_id` change entre versions | Faible | A3 fail | On lit `claude.id()` au runtime, pas hardcoded |
| `Budget` ou `LlmMetadata` champs changent (ABI) | Très faible | Compile fail | Audit du codebase à jour avant écriture |
| Test ralentit la suite CI (5 s par run) | Faible | CI plus lent | Gated par `EFE_LIVE_LLM=1` — CI normal ne lance pas |

---

## 12. Vérification post-livraison

Une fois S2 commit, on doit pouvoir reproduire exactement :

```bash
$ cd /Users/rodrigue.pekar/Downloads/vrai/jjjj copie/efe
$ export ANTHROPIC_API_KEY='sk-ant-...'
$ EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
    --test live_missions -- --nocapture

running 1 test
=== Mission A naked ===
  query    : Quelle est la capitale de la France ? Réponds en une seule phrase courte.
  llm_id   : claude_claude-sonnet-4-6
  cost_cents : 1
  latency_ms : 1245 (mission_a internal) / 1298 (wall-clock)
  audit_hash : a3f8c1b2…d7e4f3a1
  final_text : "Paris est la capitale de la France."
test live_mission_a_naked_no_slm_just_claude ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Si cette sortie est obtenue → S2 est scellé.**

---

## 13. Lien avec les sprints suivants

S2 fournit l'infrastructure (fichier `live_missions.rs`, fixtures dir, helpers
`build_factual_request`, pattern de test live) qui sera **directement réutilisée**
par S3-S9 :

- **S3** réutilise `load_fixture` + helpers, **ajoute** les robots
  analyzer/verifier via `CoordinatorClients { analyzer: Some(...), verifier: Some(...), ... }`.
- **S4** réutilise tout, **ajoute** compressor + reformulator + métriques de
  pré-mâchage.
- **S5** réutilise le pattern, **passe** à `run_mission_b` avec 3 LLMs registered.
- **S9** réutilise `build_factual_request` paramétrable pour comparer baseline
  (sans clients) vs full pipeline (tous clients).

Donc **les helpers écrits en S2 doivent être propres et réutilisables** — pas
de hardcoding qui empêcherait l'extension.

---

## 14. Conclusion

S2 est le sprint **le plus simple** de R2 mais aussi le plus **fondateur** :
sans lui, tous les sprints suivants travailleraient sur du sable.

**95 LOC de test, 10 lignes de fixture, 4 lignes de Cargo.toml = wiring
Coordinator ↔ Claude live-validé** sans nano-robots.

Une fois ce sprint vert, on peut empiler sans inquiétude les couches
verticales (S3-S9) en ayant la garantie que la fondation horizontale est
saine.

**Prochaine action** : démarrer S2.1 (création de la structure de fichiers).
