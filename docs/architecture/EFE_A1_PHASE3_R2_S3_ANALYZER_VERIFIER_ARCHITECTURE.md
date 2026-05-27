# EFE Phase 3 R2 — Sprint S3 : Mission A + analyzer + verifier

**Précédence** : S2 scellé live 12/12 (`crates/efe-coordinator/S2_LIVE_VALIDATION.md`)
**Suivant** : S4 — Mission A complète (compressor + reformulator + filter + router + `ChewingMetrics`)
**Statut** : Spec ready-to-implement
**Effort estimé** : 3 heures
**Coût $ estimé** : ~0.02 $ (Claude × 1, SLM gratuits)

---

## 0. Pourquoi ce sprint, dans cet ordre

S2 a prouvé que la **tuyauterie horizontale** (Coordinator ↔ Provider) marche
en conditions réelles. S3 ajoute les **2 nano-robots à plus haute valeur** :

- **analyzer** (SmolLM2-360M sur :8090) — *prémâchage IN* : extrait l'intent, les
  entités, la langue, le sentiment. Sa sortie nourrit la blackboard
  (phéromones d'entités) et sera consommée en S4 par filter/router.
- **verifier** (Phi-4-mini sur :8093) — *vérification OUT* : fact-check la
  réponse du LLM contre des sources fournies dans la requête. Produit un
  verdict `Supported/Unsupported/Contradicted/InsufficientEvidence`.

> **C'est le premier sprint qui prouve que les nano-robots Phase 1 et les
> gros LLMs Phase 3 collaborent réellement dans une mission.**

Ces 2 robots sont choisis avant les 4 autres parce que :
- ✅ **Indépendance** : ni l'un ni l'autre ne dépend des autres robots
- ✅ **Verticalité** : analyzer = avant LLM, verifier = après LLM → couvre les 2 phases du pré-mâchage
- ✅ **Faible coût** : SmolLM2-360M et Phi-4-mini sont gratuits (locaux), latence sub-500 ms
- ✅ **Valeur immédiate** : analyzer débloque le routing intelligent, verifier détecte les hallucinations

Les autres robots (filter / router / reformulator / compressor) apportent du
**pré-mâchage actif** (modification de la requête avant LLM) qui sera testé
collectivement en S4 avec les métriques `ChewingMetrics`.

---

## 1. Audit chirurgical pré-S3 — état exact du codebase

Audit effectué le 2026-05-27 (cf. exploration Agent dans la session R2 S3).
Tous les composants requis existent déjà — S3 est uniquement un test + 1
fixture enrichie.

### 1.1 `AnalyzerClient` — API

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/analyzer.rs:24-46`

```rust
pub struct AnalyzerClient { /* inner: LlamaServerClient */ }

impl AnalyzerClient {
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        // Charge grammar + prompt + cap max_tokens=1024 + temperature=0.1
    }
}

// trait NanoRobotClient :
async fn invoke(&self, input: AnalyzerInput, ctx: &CallContext)
    -> RobotResult<AnalyzerOutput>;
async fn health(&self) -> RobotResult<RobotHealth>;
```

**Types** :

```rust
pub struct AnalyzerInput {
    pub text: String,              // 1 ≤ len ≤ 8000
    pub max_entities: u32,         // default 20, 1 ≤ n ≤ 20
    pub source_agent: Option<String>,
}

pub struct AnalyzerOutput {
    pub entities: Vec<Entity>,         // ≤ 20
    pub intent: IntentClass,           // Factual | Analytical | Comparative | Temporal | Opinion | Instructional
    pub sentiment: f32,                // -1.0 ≤ s ≤ 1.0
    pub language: IsoLang,             // En | Fr | De | Es | It | Pt | Ja | Zh | Ru | Ar | Und
    pub one_line_summary: String,      // 1 ≤ len ≤ 200
    pub confidence: f32,               // 0.0 ≤ c ≤ 1.0
}
```

### 1.2 `VerifierClient` — API

**Fichier** : `crates/efe-nano-app-coordinator/src/clients/verifier.rs:22-46`

```rust
pub struct VerifierClient { /* inner: LlamaServerClient */ }

impl VerifierClient {
    pub fn from_assets(endpoint: impl Into<String>) -> RobotResult<Self> {
        // Charge grammar + prompt + cap max_tokens=768 + temperature=0.1
    }
}

// trait NanoRobotClient :
async fn invoke(&self, input: VerifierInput, ctx: &CallContext)
    -> RobotResult<VerifierOutput>;
async fn health(&self) -> RobotResult<RobotHealth>;
```

**Types** :

```rust
pub struct VerifierInput {
    pub claim: String,                       // 1 ≤ len ≤ 2000
    pub candidate_sources: Vec<CandidateSource>,   // **1 ≤ n ≤ 50** (obligatoire ≥1)
    pub strictness: VerifierStrictness,      // Lenient | Standard | Strict | Audit (default Standard)
}

pub struct VerifierOutput {
    pub verdict: Verdict,                    // Supported | Unsupported | Contradicted | InsufficientEvidence
    pub best_source_id: Option<String>,      // None si Unsupported
    pub similarity: f32,                     // 0.0 ≤ s ≤ 1.0
    pub contradictions: Vec<String>,         // ≤ 5 IDs
    pub severity: VerifierSeverity,          // Info | Warn | Error
    pub explanation: Option<String>,         // ≤ 500 chars
}
```

### 1.3 Comment `run_mission_a` utilise les 2 robots

**Fichier** : `crates/efe-coordinator/src/workflows/mission_a.rs`

#### Étape 1 — Analyzer (lignes 63-129)

```rust
if let Some(analyzer) = &coord.clients.analyzer {
    // Bus event : ROBOT_STARTED ("analyzer")
    let input = AnalyzerInput {
        text: req.user_text.clone(),
        max_entities: 20,
        source_agent: None,
    };
    let out = analyzer.invoke(input, &ctx).await?;
    for e in out.entities {
        let entry = BlackboardEntry::new(
            "analyzer",
            EntryKind::Entity,
            json!({"text": e.text, "kind": format!("{:?}", e.kind)}),
        );
        coord.blackboard.deposit(entry);
        // Bus event : PHEROMONE_DEPOSITED
    }
    // Bus event : ROBOT_FINISHED ("analyzer")
}
// Si analyzer est None → on continue silencieusement.
```

**Observabilité** : 3 events bus émis lorsque analyzer est actif (`ROBOT_STARTED`,
`PHEROMONE_DEPOSITED` par entité, `ROBOT_FINISHED`). Les entités sont
ajoutées à la blackboard pour consommation downstream (router/filter en S4).

#### Étape 5 — Verifier (lignes 307-352)

```rust
if !req.candidate_sources.is_empty() {
    if let Some(verifier) = &coord.clients.verifier {
        let input = VerifierInput {
            claim: llm_resp.text.clone(),
            candidate_sources: req.candidate_sources.iter()
                .map(|cs| CandidateSource { id: cs.id.clone(), text: cs.text.clone() })
                .collect(),
            strictness: VerifierStrictness::Standard,
        };
        let out = verifier.invoke(input, &ctx).await?;
        verdicts.push(FactCheck {
            claim: llm_resp.text.clone(),
            verdict: format!("{:?}", out.verdict).to_uppercase(),
            severity: format!("{:?}", out.severity).to_uppercase(),
            best_source_id: out.best_source_id,
            similarity: out.similarity,
        });
        // Bus event : VERDICT_READY
    }
}
```

**Trigger conditions** (les 2 doivent être vraies) :
1. `req.candidate_sources.is_empty() == false` (au moins 1 source fournie par l'appelant)
2. `coord.clients.verifier.is_some()` (le client est instantié)

→ **S2 a `candidate_sources: vec![]`** : verifier skip → `resp.verdicts.is_empty()` (vérifié par A8).
→ **S3 doit fournir des sources réelles** dans la requête pour que verifier tourne.

### 1.4 `CoordinatorClients` — construction partielle

**Fichier** : `crates/efe-coordinator/src/coordinator.rs:25-38`

```rust
pub struct CoordinatorClients {
    pub analyzer:     Option<Arc<AnalyzerClient>>,
    pub reformulator: Option<Arc<ReformulatorClient>>,
    pub compressor:   Option<Arc<CompressorClient>>,
    pub verifier:     Option<Arc<VerifierClient>>,
    pub filter:       Option<Arc<FilterClient>>,
    pub router:       Option<Arc<RouterClient>>,
}
```

**Construction S3** (analyzer + verifier seulement, 4 autres `None`) :

```rust
let clients = CoordinatorClients {
    analyzer:     Some(Arc::new(AnalyzerClient::from_assets("http://127.0.0.1:8090")?)),
    verifier:     Some(Arc::new(VerifierClient::from_assets("http://127.0.0.1:8093")?)),
    reformulator: None,
    compressor:   None,
    filter:       None,
    router:       None,
};
let coord = Coordinator::builder().registry(reg).clients(clients).build()?;
```

### 1.5 Events bus émis pendant Mission A avec analyzer + verifier

| Phase | Event tag | Quand |
|---|---|---|
| Start | `SWARM_STARTED` | Début mission |
| Analyzer | `ROBOT_STARTED` | Avant invoke analyzer |
| Analyzer | `PHEROMONE_DEPOSITED` | Par entité détectée (N events) |
| Analyzer | `ROBOT_FINISHED` | Après invoke analyzer |
| LLM stream | `LLM_TOKEN_STREAMED` | Par chunk de tokens |
| LLM end | `LLM_FINISHED` | Fin de la génération |
| Verifier | `VERDICT_READY` | Après invoke verifier |
| End | `SWARM_FINISHED` | Fin mission |

**Test S3 doit observer** : `SWARM_STARTED`, `ROBOT_STARTED` (analyzer), au
moins un `PHEROMONE_DEPOSITED` (l'entité "France"), `ROBOT_FINISHED`,
`LLM_FINISHED`, `VERDICT_READY`, `SWARM_FINISHED`.

### 1.6 Live tests Phase 1 — pattern existant

**Fichier** : `crates/efe-nano-app-coordinator/tests/live_smoke.rs:36-108`

```rust
fn env_url(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

if std::env::var("EFE_LIVE_SMOKE").as_deref() != Ok("1") { return; }

let client = AnalyzerClient::from_assets(env_url("ANALYZER_URL", "http://127.0.0.1:8090"))
    .expect("from_assets OK");
let health = client.health().await.expect("health call");
assert_eq!(health, RobotHealth::Up);

let input = AnalyzerInput {
    text: "Apple reported $90B revenue in Q3 2026.".into(),
    max_entities: 20,
    source_agent: Some("live_test".into()),
};
let out = client.invoke(input, &ctx).await;
```

**Convention env vars** : `ANALYZER_URL` (défaut `:8090`), `VERIFIER_URL`
(défaut `:8093`). On les réutilise en S3 pour cohérence.

### 1.7 Verdict audit

**100 % du code Rust nécessaire à S3 existe déjà**. Le sprint S3 est
uniquement :
- 1 test à écrire (~150 LOC, plus complexe que S2 à cause des sources +
  health-check des SLM)
- 1 fixture enrichie (ou nouvelle fixture `02_factual_with_sources.json`)
- 0 ligne de code source modifiée

---

## 2. Architecture S3 — Vue d'ensemble

### 2.1 Flow Mission A avec analyzer + verifier

```text
   ┌──────────────────────────────────────────────────────────────────────┐
   │  TEST live_mission_a_with_analyzer_verifier                          │
   │  EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1 + ANTHROPIC_API_KEY                  │
   │  + llama-server :8090 (analyzer) + :8093 (verifier) UP                │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  Health-check pre-flight : analyzer + verifier UP via HTTP /health    │
   │  Si DOWN → skip propre (pas de fail dur)                              │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  Coordinator::builder()                                              │
   │  ├─ registry : Claude Sonnet 4.6                                     │
   │  ├─ clients  : { analyzer: Some, verifier: Some, autres: None }      │
   │  ├─ bus      : InMemoryBus                                           │
   │  └─ blackboard : PheromoneBlackboard (capacité 256 KB)                │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionARequest {                                                   │
   │    user_text          : "Quelle est la capitale de la France ?…"     │
   │    task_class         : FactualQa                                    │
   │    available_llms     : vec![claude_id]                              │
   │    candidate_sources  : vec![                                        │
   │       { id: "wp_paris_fr", text: "Paris est la capitale de la… "},  │
   │       { id: "wp_paris_en", text: "Paris is the capital of… "},      │
   │    ]                                                                  │
   │    budget             : { max_cost_cents: 10, max_latency_ms: 60000 }│
   │    max_total_latency  : Duration::from_secs(60)                       │
   │  }                                                                    │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  run_mission_a(&coord, req).await                                    │
   │                                                                       │
   │  1. SWARM_STARTED                                                    │
   │                                                                       │
   │  2. ANALYZER (port 8090, ~150-300 ms)                                │
   │     in : { text: req.user_text, max_entities: 20 }                   │
   │     out: AnalyzerOutput { entities, intent, language, … }            │
   │     ├─ Bus : ROBOT_STARTED("analyzer")                               │
   │     ├─ Pour chaque entity : blackboard.deposit + PHEROMONE_DEPOSITED │
   │     └─ Bus : ROBOT_FINISHED("analyzer")                              │
   │                                                                       │
   │  3. filter/router/reformulator/compressor : SKIP (None en S3)        │
   │                                                                       │
   │  4. LLM choisi = available_llms.first()                              │
   │     Bridge.call(claude, prompt = req.user_text).await                │
   │     ├─ Bus : LLM_STARTED                                             │
   │     ├─ Bus : LLM_TOKEN_STREAMED × N                                  │
   │     └─ Bus : LLM_FINISHED                                            │
   │                                                                       │
   │  5. VERIFIER (port 8093, ~300-500 ms)                                │
   │     in : { claim: llm_resp.text, candidate_sources, strictness }     │
   │     out: VerifierOutput { verdict, similarity, best_source_id, … }   │
   │     ├─ Bus : VERDICT_READY                                           │
   │     └─ verdicts.push(FactCheck { verdict, severity, similarity, … }) │
   │                                                                       │
   │  6. SWARM_FINISHED                                                   │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionAResponse {                                                  │
   │    final_text        : "La capitale de la France est Paris."         │
   │    llm_used          : claude_claude-sonnet-4-6                      │
   │    cost_cents        : 1-2                                           │
   │    latency_ms        : ~2-5 s (Claude + analyzer + verifier)         │
   │    verdicts          : vec![FactCheck { verdict: "SUPPORTED", … }]   │
   │    audit_chain_hash  : "…" (64 chars BLAKE3)                         │
   │  }                                                                    │
   └──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Latence cumulée attendue

| Phase | Estimation (Mac M-series, S1 bench data) |
|---|---|
| Pre-flight health-check 2 SLM | ~50 ms |
| Analyzer invoke (8090) | 50-200 ms (court prompt) |
| Phéromone deposits (N entités) | ~10 ms |
| Claude HTTPS call | 1500-3500 ms (cf. S2) |
| Verifier invoke (8093, Phi-4-mini) | 400-800 ms (claim + 2 sources) |
| Overhead Coordinator | ~50 ms |
| **Total** | **~2.5 - 4.5 s** |

Cible test : `latency_ms < 60_000` (1 min cap large).

### 2.3 Coût attendu

| Item | Coût |
|---|---|
| Claude Sonnet 4.6 (~30 in + ~30 out tokens) | ~1-2 ¢ |
| Analyzer SmolLM2-360M (local) | 0 ¢ |
| Verifier Phi-4-mini (local) | 0 ¢ |
| **Total** | **1-2 ¢** |

---

## 3. Livrables S3 (4 fichiers, 0 ligne de code source modifiée)

### 3.1 `crates/efe-coordinator/tests/live_missions.rs` (PATCH)

Ajoute :
- **Helper** `analyzer_endpoint()` + `verifier_endpoint()` — lecture env vars
  `ANALYZER_URL` + `VERIFIER_URL` avec defaults `:8090` / `:8093`
- **Helper** `make_mission_a_coordinator_with_analyzer_verifier(provider)` —
  construit le Coordinator avec exactement les 2 robots Phase 1 instantiés
- **Helper** `slm_endpoints_alive()` — pre-flight ping HTTP /health des
  2 endpoints, retourne bool
- **Helper** `build_factual_request_with_sources(query, llm_id, sources)` —
  variante de `build_factual_request` qui inclut `candidate_sources`
- **Nouveau test** : `live_mission_a_with_analyzer_verifier` (~120 LOC)

### 3.2 `crates/efe-coordinator/tests/fixtures/missions/02_factual_with_sources.json` (NOUVEAU)

Fixture enrichie pour S3 :

```json
{
  "name": "factual_with_sources",
  "task_class": "FactualQa",
  "query": "Quelle est la capitale de la France ? Réponds en une seule phrase courte.",
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
  "expected_analyzer_intent": "Factual",
  "expected_analyzer_language": "Fr",
  "expected_analyzer_min_entities": 1,
  "comment": "Fixture S3 — étend 01_factual_simple avec sources Wikipédia pour activer le verifier. Le LLM dira 'Paris' qui matche les 2 sources, donc verdict SUPPORTED attendu avec similarity > 0.3."
}
```

### 3.3 Pas de patch Cargo.toml requis

Le test `live_mission_a_with_analyzer_verifier` est dans le binary
`live_missions` qui existe déjà depuis S2. Aucune nouvelle déclaration.

### 3.4 `docs/architecture/EFE_A1_PHASE3_R2_S3_ANALYZER_VERIFIER_ARCHITECTURE.md` (CE FICHIER)

Référence durable du contrat S3.

---

## 4. Spec détaillée du test `live_mission_a_with_analyzer_verifier`

### 4.1 Pré-conditions

| Pré-condition | Vérification | Action si fail |
|---|---|---|
| `EFE_LIVE_LLM=1` | `std::env::var("EFE_LIVE_LLM")` | `[skip]` propre |
| `EFE_LIVE_SLM=1` | `std::env::var("EFE_LIVE_SLM")` | `[skip]` propre |
| `ANTHROPIC_API_KEY` set | `ApiKey::from_env(...)` | panic explicite si EFE_LIVE_LLM=1 |
| analyzer :8090 répond /health | `curl -sf -m 2 http://127.0.0.1:8090/health` | `[skip]` propre avec message |
| verifier :8093 répond /health | idem :8093 | `[skip]` propre avec message |
| Feature `all-providers` | `#![cfg(feature = "all-providers")]` | test pas compilé sinon |

### 4.2 Comportement nominal

```text
1. Charge la fixture 02_factual_with_sources.json
2. Lit ANALYZER_URL et VERIFIER_URL depuis l'env (defaults :8090 / :8093)
3. Pre-flight : ping /health sur les 2 endpoints (skip si down)
4. Instancie :
   - ClaudeProvider::from_env() → Arc<ClaudeProvider>
   - AnalyzerClient::from_assets(analyzer_url) → Arc<AnalyzerClient>
   - VerifierClient::from_assets(verifier_url) → Arc<VerifierClient>
5. Construit ProviderRegistry avec Claude
6. Construit CoordinatorClients { analyzer: Some, verifier: Some, ... None }
7. Coordinator::builder().registry(reg).clients(clients).build()?
8. Abonne au bus (capturer events)
9. Construit MissionARequest avec :
   - user_text = fixture.query
   - candidate_sources = fixture.candidate_sources (mappés en CandidateSourceRef)
   - budget large (max_cost 10¢, max_latency 60s)
   - max_total_latency = 60s
10. resp = run_mission_a(&coord, req).await?
11. Drain les events bus
12. eprintln! résumé enrichi (intent analyzer, verdict verifier, events)
13. ASSERTIONS B1-B14 (cf. §4.3)
```

### 4.3 Assertions B1-B14

Numérotation `B*` pour distinguer des `A*` de S2 (les A* sont héritées car
toujours valides, plus on enrichit).

| # | Assertion | Justification |
|---|---|---|
| B1-B9 | (= A1-A9 de S2, héritées) | final_text, cost, latency, audit hash, wall-clock |
| B10 | `resp.verdicts.is_empty() == false` | Verifier doit avoir tourné (vs A8 de S2 qui inverse cette assertion) |
| B11 | `resp.verdicts[0].verdict == "SUPPORTED"` | Le LLM dit "Paris", les sources disent "Paris" → consensus factuel |
| B12 | `resp.verdicts[0].similarity >= 0.3` | Cosine similarity claim vs best source (Phi-4-mini sur courts textes) |
| B13 | Events bus contiennent `ROBOT_STARTED` + `ROBOT_FINISHED` (analyzer) | Preuve que le wiring analyzer marche |
| B14 | Events bus contiennent `PHEROMONE_DEPOSITED` (au moins 1) | Preuve qu'au moins une entité a été déposée (France/Paris/capitale) |
| B15 | Events bus contiennent `VERDICT_READY` | Preuve que le wiring verifier marche |
| B16 | `latency_ms < 10_000` | Cap S3 plus large que S2 (analyzer + verifier en plus) |

### 4.4 Sorties console attendues (cas nominal)

```
=== Mission A with analyzer + verifier ===
  query           : Quelle est la capitale de la France ? …
  llm_id          : claude_claude-sonnet-4-6
  analyzer_url    : http://127.0.0.1:8090
  verifier_url    : http://127.0.0.1:8093
  candidate_srcs  : 2 (wp_paris_fr, wp_france_admin)

  cost_cents      : 1
  latency_ms      : 4250 (mission_a) / 4280 (wall-clock)
  audit_hash      : a3f8…f3a1 (64 chars)
  final_text      : "La capitale de la France est Paris."

  verifier:
    verdict       : SUPPORTED
    severity      : INFO
    similarity    : 0.78
    best_source   : Some("wp_paris_fr")

  events_seen     : ["SWARM_STARTED", "ROBOT_STARTED", "PHEROMONE_DEPOSITED",
                     "PHEROMONE_DEPOSITED", "ROBOT_FINISHED", "LLM_STARTED",
                     "LLM_TOKEN_STREAMED", "LLM_TOKEN_STREAMED", "LLM_FINISHED",
                     "VERDICT_READY", "SWARM_FINISHED"]

test live_mission_a_with_analyzer_verifier ... ok
test result: ok. 2 passed; 0 failed
```

### 4.5 Comportement en cas d'absence des variables

```bash
# Sans EFE_LIVE_LLM (et/ou EFE_LIVE_SLM) :
$ cargo test ... --test live_missions
running 2 tests
test live_mission_a_naked_no_slm_just_claude ... ok    # skip propre (S2)
test live_mission_a_with_analyzer_verifier ... ok      # skip propre (S3)
test result: ok. 2 passed; 0 failed
```

```bash
# Avec EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1 mais SLM down :
$ EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test ... --test live_missions
running 2 tests
test live_mission_a_naked_no_slm_just_claude ... ok    # S2 tourne normalement (pas besoin SLM)
test live_mission_a_with_analyzer_verifier ... [skip] analyzer or verifier not responding on :8090/:8093
test result: ok. 2 passed; 0 failed
```

```bash
# Cas nominal :
$ EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 ANTHROPIC_API_KEY=sk-ant-... cargo test ...
running 2 tests
test live_mission_a_naked_no_slm_just_claude ... ok   # 11/11 assertions (S2)
test live_mission_a_with_analyzer_verifier ... ok    # 16/16 assertions (S3)
test result: ok. 2 passed; 0 failed
```

---

## 5. Code Rust complet du test S3 (~150 LOC)

```rust
// (Ajouts à live_missions.rs après le test S2)

// ─── Helpers S3 ──────────────────────────────────────────────────────────

/// URL de l'analyzer (port 8090). Convention env `ANALYZER_URL` ou défaut.
fn analyzer_endpoint() -> String {
    std::env::var("ANALYZER_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

/// URL du verifier (port 8093). Convention env `VERIFIER_URL` ou défaut.
fn verifier_endpoint() -> String {
    std::env::var("VERIFIER_URL").unwrap_or_else(|_| "http://127.0.0.1:8093".to_string())
}

/// Gate spécifique aux tests qui requièrent les SLM Phase 1.
/// Lit `EFE_LIVE_SLM=1` (en plus de `EFE_LIVE_LLM`).
fn live_slm_enabled() -> bool {
    std::env::var("EFE_LIVE_SLM").as_deref() == Ok("1")
}

/// Vérifie que les 2 endpoints SLM répondent à /health. Retourne true si
/// les 2 sont UP, false sinon. Non bloquant — l'appelant skip si false.
async fn analyzer_and_verifier_alive() -> bool {
    let client = reqwest::Client::new();
    let analyzer_ok = client
        .get(format!("{}/health", analyzer_endpoint()))
        .timeout(Duration::from_secs(2))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    let verifier_ok = client
        .get(format!("{}/health", verifier_endpoint()))
        .timeout(Duration::from_secs(2))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    analyzer_ok && verifier_ok
}

/// Variante de `build_factual_request` qui inclut des candidate_sources
/// (requis pour activer le verifier en S3+).
fn build_factual_request_with_sources(
    query: &str,
    llm_id: &str,
    sources: Vec<(String, String)>,   // (id, text)
) -> MissionARequest {
    let mut metadata = HashMap::new();
    metadata.insert(llm_id.to_string(), default_claude_metadata());
    MissionARequest {
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            max_cost_cents: 10,
            max_latency_ms: 60_000,   // élargi vs S2 car analyzer+verifier ajoutent ~1s
            min_quality_score: 0.0,
        },
        candidate_sources: sources.into_iter().map(|(id, text)| {
            efe_coordinator::CandidateSourceRef { id, text }
        }).collect(),
        max_total_latency: Duration::from_secs(60),
    }
}

/// Construit un Coordinator avec EXACTEMENT analyzer + verifier (autres None).
///
/// Variante "partial" de make_naked_coordinator pour Sprint S3. Sera
/// remplacée en S4 par make_full_coordinator (6 robots).
fn make_coord_with_analyzer_verifier(
    provider: Arc<dyn LlmProvider>,
) -> efe_coordinator::CoordResult<Coordinator> {
    use efe_coordinator::CoordinatorClients;
    use efe_nano_app_coordinator::clients::{AnalyzerClient, VerifierClient};

    let analyzer = Arc::new(AnalyzerClient::from_assets(analyzer_endpoint())
        .map_err(|e| efe_coordinator::CoordError::Config(format!("analyzer init: {e}")))?);
    let verifier = Arc::new(VerifierClient::from_assets(verifier_endpoint())
        .map_err(|e| efe_coordinator::CoordError::Config(format!("verifier init: {e}")))?);

    let clients = CoordinatorClients {
        analyzer:     Some(analyzer),
        verifier:     Some(verifier),
        reformulator: None,
        compressor:   None,
        filter:       None,
        router:       None,
    };

    let registry = ProviderRegistry::new();
    registry.register(provider);

    Coordinator::builder()
        .registry(registry)
        .clients(clients)
        .build()
}

// ─── Sprint S3 — Mission A + analyzer + verifier ─────────────────────────

#[tokio::test]
async fn live_mission_a_with_analyzer_verifier() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY");
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 (+ llama-server :8090/:8093 UP)");
        return;
    }
    if !analyzer_and_verifier_alive().await {
        eprintln!("[skip] analyzer (:8090) or verifier (:8093) not responding on /health");
        eprintln!("       → lance d'abord : ./scripts/slm/start.sh && ./scripts/slm/health.sh");
        return;
    }

    // 1. Charge la fixture enrichie
    let fixture = load_fixture("02_factual_with_sources.json");
    let query = fixture["query"].as_str().expect("fixture.query missing");
    let expected_kw = fixture["expected_keyword"].as_str().expect("fixture.expected_keyword missing");
    let expected_verdict = fixture["expected_verifier_verdict"]
        .as_str().unwrap_or("SUPPORTED");
    let expected_sim_min: f32 = fixture["expected_verifier_similarity_min"]
        .as_f64().unwrap_or(0.3) as f32;
    let sources: Vec<(String, String)> = fixture["candidate_sources"]
        .as_array().expect("fixture.candidate_sources")
        .iter().map(|s| (
            s["id"].as_str().unwrap().to_string(),
            s["text"].as_str().unwrap().to_string(),
        )).collect();
    assert!(!sources.is_empty(), "fixture must provide ≥1 candidate_source");

    // 2. Provider Claude réel
    let claude = Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let claude_id = claude.id();

    // 3. Coordinator avec analyzer + verifier
    let coord = make_coord_with_analyzer_verifier(claude)
        .expect("coord builds with analyzer + verifier");

    eprintln!("=== Mission A with analyzer + verifier ===");
    eprintln!("  query          : {query}");
    eprintln!("  llm_id         : {claude_id}");
    eprintln!("  analyzer_url   : {}", analyzer_endpoint());
    eprintln!("  verifier_url   : {}", verifier_endpoint());
    eprintln!("  candidate_srcs : {} ({:?})", sources.len(),
        sources.iter().map(|(id, _)| id).collect::<Vec<_>>());

    // 4. Subscribe events bus
    let (_token, mut rx) = coord.subscribe(Some("s3-test"));

    // 5. Build request
    let req = build_factual_request_with_sources(query, &claude_id, sources.clone());

    // 6. Run mission
    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req).await
        .expect("mission_a should succeed (HTTP 200, analyzer up, verifier up, audit OK)");
    let wall_ms = started.elapsed().as_millis() as u64;

    // 7. Drain events (timeout 100 ms — plus large que S2 car plus d'events)
    let mut tags: Vec<&'static str> = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
        tags.push(ev.tag());
    }

    // 8. Affichage
    let v0 = resp.verdicts.first();
    eprintln!("  cost_cents     : {}", resp.cost_cents);
    eprintln!("  latency_ms     : {} / wall-clock {}", resp.latency_ms, wall_ms);
    eprintln!("  final_text     : {:?}", resp.final_text);
    if let Some(v) = v0 {
        eprintln!("  verifier:");
        eprintln!("    verdict      : {}", v.verdict);
        eprintln!("    severity     : {}", v.severity);
        eprintln!("    similarity   : {:.2}", v.similarity);
        eprintln!("    best_source  : {:?}", v.best_source_id);
    }
    eprintln!("  events_seen ({}) : {tags:?}", tags.len());

    // 9. Assertions B1-B16 (cf. §4.3)
    assert!(!resp.final_text.is_empty(), "B1");
    assert!(resp.final_text.to_lowercase().contains(expected_kw),
        "B2: contient '{expected_kw}', got {:?}", resp.final_text);
    assert_eq!(resp.llm_used, claude_id, "B3");
    assert!(resp.cost_cents <= 5, "B4 cost ≤ 5¢, got {}¢", resp.cost_cents);
    assert!(resp.latency_ms >= 100, "B6 latency ≥ 100 ms");
    assert_eq!(resp.audit_chain_hash.len(), 64, "B7 audit_chain_hash 64 chars");
    assert!(wall_ms < 60_000, "B9 wall-clock < 60s");
    assert!(resp.latency_ms < 60_000, "B16 latency < 60s");

    // B10 — verifier a tourné (verdicts non vide)
    assert!(!resp.verdicts.is_empty(), "B10: verifier doit avoir produit ≥1 verdict");
    let v = &resp.verdicts[0];

    // B11 — verdict attendu
    assert_eq!(v.verdict.to_uppercase(), expected_verdict.to_uppercase(),
        "B11: verdict attendu '{expected_verdict}', got '{}'", v.verdict);

    // B12 — similarity au-dessus du seuil
    assert!(v.similarity >= expected_sim_min,
        "B12: similarity ≥ {expected_sim_min}, got {}", v.similarity);

    // B13-B15 — events analyzer + verifier observés
    assert!(tags.contains(&"ROBOT_STARTED"), "B13a ROBOT_STARTED missing: {tags:?}");
    assert!(tags.contains(&"ROBOT_FINISHED"), "B13b ROBOT_FINISHED missing");
    assert!(tags.contains(&"PHEROMONE_DEPOSITED"),
        "B14 PHEROMONE_DEPOSITED missing (analyzer doit déposer ≥1 entité)");
    assert!(tags.contains(&"VERDICT_READY"), "B15 VERDICT_READY missing");
}
```

**Total : ~150 LOC.** Aucune dépendance interne supplémentaire.

---

## 6. Plan d'implémentation chirurgical S3

7 sub-sprints de 15-30 min, committables indépendamment :

### S3.1 — Fixture enrichie `02_factual_with_sources.json` (10 min)

- Créer le fichier
- Vérifier `python3 -m json.tool < ... > /dev/null` valide

**Critère** : JSON valide + champs requis présents (query, expected_keyword,
candidate_sources avec ≥1 source, expected_verifier_verdict).

### S3.2 — Helpers S3 dans `live_missions.rs` (25 min)

Ajouter dans `tests/live_missions.rs` (après les helpers S2) :
- `analyzer_endpoint()`, `verifier_endpoint()`, `live_slm_enabled()`
- `analyzer_and_verifier_alive()` (async, reqwest pour /health)
- `build_factual_request_with_sources()`
- `make_coord_with_analyzer_verifier()`

**Critère** : `cargo check -p efe-coordinator --features all-providers --tests`
sans erreur ni warning.

### S3.3 — Le test minimal sans assertions verifier (20 min)

Écrire le squelette du test avec :
- Tous les early-returns (live_enabled, live_slm_enabled, analyzer_and_verifier_alive)
- Chargement fixture
- Run mission_a
- Assertions B1-B9 (héritées S2) + B16 (latency)
- Assertions B13-B15 sur events
- **Pas encore** B10-B12 (verifier-specific) pour valider que la mission tourne

**Critère** : skip propre OK. Si on a Claude + SLM up, le test PASS sur
B1-B9 + B16 + B13-B15.

### S3.4 — Assertions verifier B10-B12 (15 min)

Ajouter les 3 assertions sur les verdicts (non vide, verdict SUPPORTED,
similarity ≥ seuil).

**Critère** : test PASS toutes les 16 assertions sur fixture 02.

### S3.5 — Test live réel sur la machine du user (20 min)

```bash
just slm-health                          # vérifier daemons UP
just live-check-keys --required anthropic
EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 \
  cargo test -p efe-coordinator --features all-providers \
    --test live_missions live_mission_a_with_analyzer_verifier -- --nocapture
```

**Critère** : test PASS, sortie console conforme §4.4.

### S3.6 — Vérification non-régression (10 min)

```bash
cargo test -p efe-coordinator --features all-providers --tests  # 395 → 396
cargo clippy -p efe-coordinator --features all-providers --tests --no-deps  # 0 warning S3
```

**Critère** : 396/396 PASS, 0 warning sur le code S3.

### S3.7 — `S3_LIVE_VALIDATION.md` + commit final (20 min)

- Créer `crates/efe-coordinator/S3_LIVE_VALIDATION.md` (analogue S2)
- Mettre à jour `tests/fixtures/missions/README.md` (référencer fixture 02)
- Commit `feat(phase-3-r2-s3): Mission A + analyzer + verifier live-validated`

**Total temps** : ~2 h cumulés + buffer 1 h imprévus = **3 heures**.

---

## 7. Critères d'acceptation binaires S3

S3 est **livré** quand TOUS ces critères sont vrais :

- [ ] `tests/fixtures/missions/02_factual_with_sources.json` existe et valide
- [ ] `tests/live_missions.rs` contient les 5 helpers S3 + le test
  `live_mission_a_with_analyzer_verifier`
- [ ] `cargo build -p efe-coordinator --features all-providers --tests` clean
- [ ] `cargo clippy --tests --no-deps` : 0 warning sur le code S3
- [ ] Skip propre sans `EFE_LIVE_LLM=1` OU sans `EFE_LIVE_SLM=1` OU sans
  SLM UP : `1 passed` (skip silencieux)
- [ ] Test live PASS avec env + SLM up : **16/16 assertions B1-B15 vertes**
- [ ] Coût observé ≤ 3 ¢
- [ ] Latence observée < 10 000 ms
- [ ] Verdict observé == `SUPPORTED` (pas Unsupported/Contradicted)
- [ ] Similarity observée ≥ 0.3
- [ ] **0 régression** sur la suite offline : 396/396 PASS
- [ ] `S3_LIVE_VALIDATION.md` scellé avec valeurs observées
- [ ] Commit final pushe avec message `feat(phase-3-r2-s3): …`

---

## 8. Hors-scope S3 (présents dans S4-S9)

| Item | Sprint cible | Pourquoi pas S3 |
|---|---|---|
| Compressor (`compressor.gguf` :8092) | S4 | Pré-mâchage actif = modification de la requête |
| Reformulator (`reformulator.gguf` :8091) | S4 | idem |
| Filter (`bge-m3` ONNX local) | S4 | Routing/ranking, dépend de l'analyzer |
| Router (Pareto heuristique) | S4 + tuning S6 | Sélection LLM (S3 force Claude) |
| `ChewingMetrics` (tokens before/after) | S4 | Mesure du gain de pré-mâchage |
| Mission B (3-LLM vote) | S5 | Multi-provider |
| Failure modes (SLM crash, LLM 429) | S8 | Tests négatifs |
| Multiple fixtures (5+) | S9 | Savings report |

**Anti-pattern à éviter en S3** : si on commence à ajouter "juste le router
parce qu'on en a besoin pour S4", on perd la valeur de l'incrémentalité.
S3 garde strictement 2 robots (analyzer + verifier).

---

## 9. Risques connus & mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Analyzer langue détection != Fr (output "Und") | Faible | B14 cosmétique (event présence, pas valeur) | Pas d'assertion sur language exact en S3 |
| Verifier verdict `InsufficientEvidence` | Moyenne | B11 fail | Choisir des sources qui mentionnent explicitement "Paris" (cas fixture 02 OK) |
| Verifier similarity < 0.3 (Phi-4-mini parfois conservatif) | Moyenne | B12 fail | Seuil min 0.3 (très bas, marge confortable) |
| Latence cumulée > 60s (réseau lent, modèle saturé) | Faible | B9/B16 fail | Cap large 60s + Mac M Metal est rapide |
| Phi-4-mini consomme trop de RAM → swap | Faible | latence × 5 | RAM check en S1 + warning utilisateur si dispo < 6 GB |
| `expected_verdict` du fixture évolue avec versions modèles | Faible | B11 fail futur | Verdict capable de varier entre Supported/InsufficientEvidence — accepter les 2 ? À discuter |
| Bug compile dans `make_coord_with_analyzer_verifier` (import path) | Très faible | compile fail | Audit chirurgical déjà fait — chemins vérifiés |

---

## 10. Vérification post-livraison

Une fois S3 commit, on doit pouvoir reproduire :

```bash
$ ./scripts/slm/start.sh                                    # 4 daemons UP
$ ./scripts/slm/health.sh                                    # 4/4 ● UP
$ ./scripts/slm/check_live_env.sh --required anthropic       # ✅ clé OK
$ EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 \
    cargo test -p efe-coordinator --features all-providers \
      --test live_missions -- --nocapture

running 2 tests

=== Mission A naked ===
...
test live_mission_a_naked_no_slm_just_claude ... ok       # S2 inchangé

=== Mission A with analyzer + verifier ===
  query           : Quelle est la capitale de la France ? …
  analyzer_url    : http://127.0.0.1:8090
  verifier_url    : http://127.0.0.1:8093
  candidate_srcs  : 2 (wp_paris_fr, wp_france_admin)
  cost_cents      : 1
  latency_ms      : ~4200 / wall ~4250
  audit_hash      : … (64)
  final_text      : "La capitale de la France est Paris."
  verifier:
    verdict       : SUPPORTED
    similarity    : 0.78
    best_source   : Some("wp_paris_fr")
  events_seen (11): [SWARM_STARTED, ROBOT_STARTED, PHEROMONE_DEPOSITED, ...,
                     LLM_FINISHED, VERDICT_READY, SWARM_FINISHED]
test live_mission_a_with_analyzer_verifier ... ok

test result: ok. 2 passed; 0 failed
```

**Si cette sortie est obtenue → S3 est scellé.**

---

## 11. Lien avec les sprints suivants

S3 pose **3 fondations critiques** pour S4-S9 :

### Pour S4 (Mission A complète)
- `make_coord_with_*` template extensible : `make_coord_full` ajoutera les 4
  robots restants
- Convention des env vars `<ROBOT>_URL` validée → S4 ajoutera
  `COMPRESSOR_URL`, `REFORMULATOR_URL`
- `analyzer_and_verifier_alive` → généralisé en `all_slm_alive` qui pingera 4
  endpoints

### Pour S5 (Mission B 3-LLM vote)
- Pattern de capture des events bus + assertions sur les tags : duplicable
  pour les 3 LLMs en parallèle

### Pour S8 (Failure modes)
- `analyzer_and_verifier_alive` retourne false → on a un test qui prouve la
  dégradation gracieuse. On ajoutera en S8 un test qui simule analyzer DOWN
  alors qu'il était UP au démarrage (kill -9 pendant le run)

### Pour S9 (Chewing savings report)
- Fixture `02_factual_with_sources.json` enrichit déjà la convention format
  → S9 ajoutera 4 fixtures supplémentaires en suivant le même schéma

---

## 12. Conclusion

S3 est le sprint qui prouve **que les nano-robots Phase 1 et les gros LLMs
Phase 3 collaborent réellement dans une mission live**. C'est le pas charnière
entre "tuyauterie horizontale" (S2) et "pré-mâchage complet" (S4-S9).

**~150 LOC de test, 22 lignes de fixture, 0 modification du code source =
chaîne analyzer → Claude → verifier live-validée** avec 16 assertions.

Une fois ce sprint vert, on peut empiler S4 (compressor + métriques)
sans inquiétude sur la base.

**Prochaine action** : démarrer S3.1 (fixture `02_factual_with_sources.json`).
