# EFE Phase 3 R2 — Sprint S5 : Mission B (3-LLM Consensus + Pré-mâchage Mutualisé)

**Précédence** : S4 scellé live 28/28 (`crates/efe-coordinator/S4_LIVE_VALIDATION.md`)
                 → **85 % savings tokens prouvés sur 1 LLM**
**Suivant** : S6 — Router tuning par task_class
**Statut** : Spec ready-to-implement
**Effort estimé** : 4 heures
**Coût $ estimé** : ~0.10 $ (3 LLMs × ~2-3 ¢ chacun en parallèle)

---

## 0. Pourquoi ce sprint — la multiplication économique

S4 a démontré **85 % de savings tokens** sur **1 LLM** (Claude). S5 prouve que
**ce gain se MULTIPLIE quand on appelle plusieurs LLMs gros** :

> **1 compresseur × N gros LLMs = savings × N** (sans coût SLM supplémentaire,
> car le SLM tourne 1 seule fois).

Concrètement :
- En **S4** : compressor 1× → Claude reçoit 20 tokens (vs 132 raw) → savings sur 1 LLM
- En **S5** : compressor 1× → **Claude + Gemini + GPT reçoivent CHACUN 20 tokens** (vs
  132×3 = 396 raw si on n'avait pas le compressor) → **savings × 3**

C'est l'**argument économique mutualisé** : plus on consulte de LLMs en
parallèle (Mission B), plus le pré-mâchage est rentable. EFE devient
asymptotiquement **N× moins cher qu'une approche naïve** quand N grandit.

S5 valide ce gain en live avec les **3 vrais providers** (Claude Sonnet 4.6
+ Gemini 2.5 Flash + GPT-4o) — les 3 mêmes que validés en R1b (vote 3/3
unanime sur "Paris").

---

## 1. Audit chirurgical pré-S5 — état exact du codebase

Audit effectué le 2026-05-27 post-S4. **Découverte majeure** ci-après.

### 1.1 `MissionBRequest` / `MissionBResponse`

**Fichier** : `crates/efe-coordinator/src/request.rs:78-140`

```rust
pub struct MissionBRequest {
    pub user_text: String,
    pub task_class: TaskClass,
    pub llms: Vec<String>,                              // ≥ 2 requis (sinon erreur)
    pub llm_metadata: HashMap<String, LlmMetadata>,
    pub vote_weights: HashMap<String, f32>,             // optionnel, défaut uniform 1.0
    pub quorum: Option<usize>,                          // défaut ceil(|llms| / 2)
    pub candidate_sources: Vec<CandidateSourceRef>,
    pub max_total_latency: Duration,                    // défaut 30 s
}

pub struct MissionBResponse {
    pub final_text: String,
    pub winner_llm: String,
    pub winner_score: f32,                              // somme pondérée
    pub per_llm_scores: HashMap<String, f32>,
    pub total_cost_cents: u32,                          // ⚠ SOMME des 3 LLMs
    pub latency_ms: u64,
    pub verdicts: Vec<FactCheck>,
    pub audit_chain_hash: String,
    // ❌ Pas de chewing_metrics — à ajouter en S5
}
```

**Découverte importante** : `total_cost_cents` est **la somme** des 3 LLMs, pas
le coût du gagnant. C'est cohérent avec le modèle économique : on PAIE les 3
appels même si on n'en garde qu'un (après vote). Le savings via compressor
réduit les **3 coûts** simultanément.

### 1.2 `run_mission_b` workflow — 5 étapes

**Fichier** : `crates/efe-coordinator/src/workflows/mission_b.rs:24-283`

```rust
pub async fn run_mission_b(
    coord: &Coordinator,
    req: MissionBRequest,
) -> CoordResult<MissionBResponse> {
    // ── Phase 1 (lignes 50-92) — Pré-mâchage MUTUALISÉ ─────────
    //    analyzer → compressor → prompt_for_llms (1× shared!)
    //    Le compressor tourne UNE SEULE FOIS. Son output est le prompt
    //    qui sera envoyé en parallèle aux N LLMs.

    // ── Phase 2 (lignes 95-130) — Fan-out parallèle ────────────
    let responses: Vec<LlmResponse> = coord
        .bridge
        .stream_many(&req.llms, llm_call, chunk_tx)
        .await
        .into_iter()
        .filter_map(Result::ok)  // skip silencieusement les erreurs
        .collect();

    // ── Phase 3 (lignes 138-165) — Vote Symphony ───────────────
    let ballots: Vec<Ballot> = responses.iter().map(|r| Ballot {
        provider: r.provider.clone(),
        answer_text: r.text.clone(),
        self_quality: r.self_quality,
        latency_ms: r.latency_ms,
    }).collect();
    let outcome = VoteEngine::new(
        req.vote_weights.clone(),
        success_rates_from_metadata(&req.llm_metadata),
    ).tally(&ballots, quorum)?;
    bus.publish(StreamEvent::VoteResolved { winner, scores, ... }).await;

    // ── Phase 4 (lignes 167-196) — Verifier optionnel ──────────
    if !req.candidate_sources.is_empty() {
        if let Some(verifier) = &coord.clients.verifier {
            verifier.invoke(VerifierInput {
                claim: outcome.winner_text.clone(),
                ...
            }).await?;
        }
    }

    // ── Phase 5 (lignes 198-241) — Audit + Response ────────────
    let total_cost: u32 = responses.iter().map(|r| r.cost_cents).sum();
    Ok(MissionBResponse {
        final_text: outcome.winner_text,
        winner_llm: outcome.winner,
        winner_score: outcome.winner_score,
        per_llm_scores: outcome.per_provider_scores,
        total_cost_cents: total_cost,  // SOMME des 3 LLMs
        latency_ms: started.elapsed().as_millis() as u64,
        verdicts,
        audit_chain_hash: audit_hash,
    })
}
```

**Constat** : Le workflow Mission B fait **déjà** le pré-mâchage mutualisé
correctement. Le compressor tourne 1× ligne ~80, son output est passé au
fan-out ligne ~100. Aucune modification structurelle nécessaire.

### 1.3 `VoteEngine` Symphony — algorithme

**Fichier** : `crates/efe-coordinator/src/vote/engine.rs:1-330`

```rust
pub struct Ballot {
    pub provider: String,
    pub answer_text: String,
    pub self_quality: f32,        // 0..1 — auto-évaluation du provider
    pub latency_ms: u64,
}

pub struct VoteOutcome {
    pub winner: String,                              // ID du LLM gagnant
    pub winner_text: String,                         // texte de réponse winning
    pub winner_score: f32,                           // somme pondérée
    pub per_provider_scores: HashMap<String, f32>,
    pub quorum_required: usize,
    pub ballots_received: usize,
}

impl VoteEngine {
    pub fn tally(&self, ballots: &[Ballot], quorum: usize)
        -> CoordResult<VoteOutcome>;
}
```

**Algorithme 5 phases** (cf. `engine.rs:78-280`) :
1. **Score per-provider** : `score = weight × self_quality`
2. **Clustering** : groupe les ballots par texte canonicalisé
   (BLAKE3 hash de `lowercase().replace(whitespace+, " ")`)
3. **Sélection cluster max** : somme des scores ; tie-break par min(provider) lexicographique
4. **Check quorum** : si `votes_pour_winner < quorum` → fallback **Invariant I5**
   sur le provider avec max(success_rate)
5. **VoteOutcome**

### 1.4 `MultiLlmBridge.stream_many` — fan-out parallèle

**Fichier** : `crates/efe-coordinator/src/llm/bridge.rs:75-130`

```rust
pub async fn stream_many(
    &self,
    ids: &[String],
    req: LlmCall,
    chunk_tx: mpsc::Sender<TokenChunk>,
) -> Vec<CoordResult<LlmResponse>> {
    // FuturesUnordered : démarre les N appels en parallèle
    // Chaque chunk_tx envoie le tag du provider + token streamé
    // Latence = max(call_n), pas somme
}
```

**Comportement clé** : latence parallèle = `max(call_n)`, **pas la somme**.
C'est ce qui permet le savings latence : 3 LLMs en parallèle = 1× max
latence, **pas 3× cumulé**.

### 1.5 `StreamEvent::VoteResolved` — déjà présent

```rust
VoteResolved {
    swarm_id: Uuid,
    winner: String,
    winner_score: f32,
    per_llm_scores: HashMap<String, f32>,
    ts: DateTime<Utc>,
}
// tag() = "VOTE_RESOLVED"
```

Émis dans `mission_b.rs:155`. ✅ pas besoin de l'ajouter.

### 1.6 Tests offline workflow_mission_b — 5 tests existants

**Fichier** : `crates/efe-coordinator/tests/workflow_mission_b.rs`

1. `mission_b_with_unanimous_llms_picks_consensus` — 2 LLMs même réponse
2. `mission_b_too_few_llms_errors` — `< 2 LLMs` → erreur
3. `mission_b_all_providers_fail_errors` — tous fail → erreur
4. `mission_b_publishes_vote_resolved_event` — vérifie event
5. `mission_b_weighted_vote_overrides_majority` — vote pondéré

**Gap S5** : Aucun test ne vérifie `chewing_metrics` ni `total_cost_cents`
sommé. S5 ajoutera ces tests offline (similaire au pattern G3 de S4).

### 1.7 Test live R1b `live_vote_3llm_real_claude_gemini_gpt`

**Fichier** : `crates/efe-coordinator/tests/live_providers.rs:265-319`

Ce test utilise **directement** `bridge.call_many_ok` — **PAS** `run_mission_b`.
Il ne passe pas par le workflow complet (pas d'analyzer, pas de compressor,
pas de vote Symphony — juste filtrage des résultats). C'est exactement ce
que **S5 vient combler** : passer par le pipeline mission complet.

### 1.8 Verdict audit

| Composant | Statut |
|---|---|
| `MissionBRequest` / `MissionBResponse` | ✅ existants |
| `run_mission_b` workflow 5 phases | ✅ existant et fait déjà le pré-mâchage mutualisé |
| `VoteEngine::tally` Symphony | ✅ existant et testé offline |
| `MultiLlmBridge.stream_many` fan-out | ✅ existant, parallèle FuturesUnordered |
| `StreamEvent::VoteResolved` | ✅ existant, émis par le workflow |
| Tests offline Mission B (5) | ✅ existants |
| Test live 3-LLM via workflow Mission B | ❌ **manquant — gap S5** |
| `chewing_metrics` dans `MissionBResponse` | ❌ **manquant — gap S5** |
| Émission `PreChewingMetrics` dans `run_mission_b` | ❌ **manquant — gap S5** |
| Tests offline pour chewing_metrics Mission B | ❌ **manquant — gap S5** |

**Conclusion** : S5 = **3 modifications source légères** + **1 test live** +
**3 tests offline** + **1 fixture (réutiliser 03_factual_compressible)**.

---

## 2. Architecture S5 — Vue d'ensemble

### 2.1 Flow Mission B avec pré-mâchage mutualisé + vote Symphony

```text
   ┌──────────────────────────────────────────────────────────────────────┐
   │  TEST live_mission_b_full_3llm_consensus_with_pre_chewing            │
   │  EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1                                      │
   │  + ANTHROPIC_API_KEY + GOOGLE_AI_KEY + OPENAI_API_KEY                 │
   │  + llama-server :8090/:8092/:8093 UP                                  │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  Coordinator avec 3 providers + 3 nano-robots :                      │
   │  registry = { claude, gemini, gpt }                                  │
   │  clients  = { analyzer + compressor + verifier + router : Some,      │
   │               reformulator + filter : None }                          │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionBRequest {                                                   │
   │    user_text          : "<paragraphe long ≥ 500 chars>"              │
   │    llms               : ["claude_…", "gemini_…", "gpt_…"]            │
   │    vote_weights       : {}  (uniform 1.0 chacun)                     │
   │    quorum             : None (= ceil(3/2) = 2)                        │
   │    candidate_sources  : [<src1>, <src2>]                              │
   │  }                                                                    │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  run_mission_b (modifié S5) :                                        │
   │                                                                       │
   │  1. SWARM_STARTED (mission=MissionB)                                 │
   │                                                                       │
   │  2. ANALYZER  → entities (idem S4)                                   │
   │     event ROBOT_STARTED + PHEROMONE_DEPOSITED ×N + ROBOT_FINISHED    │
   │                                                                       │
   │  3. COMPRESSOR (1× SEULEMENT !) → prompt_compressé                   │
   │     event ROBOT_STARTED + ROBOT_FINISHED                              │
   │     ► Le SAVINGS s'applique MUTUELLEMENT aux 3 LLMs en aval ◄        │
   │                                                                       │
   │  4. FAN-OUT 3 LLMs (parallèle, latence = max) :                      │
   │       ┌──────────┬──────────┬──────────┐                             │
   │       ▼          ▼          ▼                                        │
   │     Claude    Gemini      GPT-4o                                     │
   │     ~3s       ~1s         ~2s                                        │
   │       │          │          │                                        │
   │       └──────────┴──────────┘                                        │
   │     events LLM_TOKEN_STREAMED ×N + LLM_FINISHED ×3                   │
   │                                                                       │
   │  5. VOTE SYMPHONY :                                                  │
   │     VoteEngine::tally(ballots, quorum=2)                             │
   │     event VOTE_RESOLVED { winner, scores }                            │
   │                                                                       │
   │  6. VERIFIER → fact-check du winner_text                             │
   │     event VERDICT_READY                                              │
   │                                                                       │
   │  7. CHEWING METRICS (S5 NEW) :                                       │
   │     metrics.input_tokens_raw = estimate(user_text)                   │
   │     metrics.input_tokens_compressed = estimate(prompt_compressé)     │
   │     metrics.compression_ratio = compressed/raw                       │
   │     metrics.savings_tokens × 3 (mutualisé !)                         │
   │     metrics.llm_latency_ms = max(claude, gemini, gpt)                │
   │     metrics.llm_tokens_generated = somme des 3                       │
   │     metrics.selected_llm = winner_llm                                │
   │     event PRE_CHEWING_METRICS                                        │
   │                                                                       │
   │  8. SWARM_FINISHED                                                   │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │  MissionBResponse {                                                  │
   │    final_text       : "Paris est…" (texte du LLM gagnant)            │
   │    winner_llm       : "claude_…" (par exemple)                       │
   │    winner_score     : 1.0 (uniform)                                  │
   │    per_llm_scores   : {claude: 1.0, gemini: 1.0, gpt: 1.0}           │
   │    total_cost_cents : 3 (= 1+1+1 sommé)                              │
   │    latency_ms       : ~14_000 (~SLM + max(3 LLMs))                   │
   │    verdicts         : [SUPPORTED, …]                                 │
   │    audit_chain_hash : "…" (64 BLAKE3)                                │
   │    chewing_metrics  : Some(ChewingMetrics { savings 85% × 3 })  ◄ S5 │
   │  }                                                                    │
   └──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Multiplication économique attendue

| Scénario | Tokens d'entrée envoyés au LLM cumulés | Coût LLM | Coût SLM |
|---|---|---|---|
| **Baseline naïf** (3 LLMs sans pré-mâchage) | 132 × 3 = **396** | ~6-9 ¢ | 0 ¢ |
| **S5 avec pré-mâchage mutualisé** | 20 × 3 = **60** | ~3 ¢ | 0 ¢ (local) |
| **Savings** | **−336 tokens (−85 %)** | **−50 à 60 %** | — |

**Asymptotiquement** : si on appelait 10 LLMs en parallèle, savings = 1120 tokens cumulés
vs 1 compressor unique → ratio savings × 10. C'est l'**effet multiplicatif** de R2.

### 2.3 Vote 3/3 attendu (consensus factuel)

Pour la query "Quelle est la capitale de la France ?", les 3 LLMs vont
**unanimement** répondre "Paris" (validé en R1b live). VoteEngine va :
1. Hash canonicalisé de "Paris" identique pour les 3 → 1 cluster
2. Score cluster = 1.0 + 1.0 + 1.0 = 3.0 (uniform)
3. Tie-break inutile (1 seul cluster)
4. Quorum = 2, votes = 3 → quorum atteint
5. Winner = min(provider lex) = `claude_…` (par défaut lex)

---

## 3. Livrables S5 (7 fichiers — 3 src modifiés + 4 nouveaux)

### 3.1 `src/request.rs` (PATCH +5 LOC)

Ajout du champ identique à MissionAResponse :

```rust
pub struct MissionBResponse {
    // ... (champs existants) ...
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,
}
```

### 3.2 `src/workflows/mission_b.rs` (PATCH ~60 LOC)

Instrumenter latences par phase + agrégats fan-out + émettre `PreChewingMetrics` event + remplir `MissionBResponse.chewing_metrics`.

Différences vs S4 mission_a :
- **`llm_latency_ms`** = `max(responses.iter().map(|r| r.latency_ms))` (parallèle)
- **`llm_tokens_generated`** = `responses.iter().map(|r| r.tokens_generated).sum()` (somme des 3)
- **`selected_llm`** = `outcome.winner` (après vote, pas avant)
- **`router_*`** = vides (Mission B n'utilise pas le router — les N LLMs sont tous appelés)

### 3.3 `tests/live_missions.rs` (PATCH ~200 LOC)

- Helper `make_coord_for_mission_b_pipeline(claude, gemini, gpt)` : registry 3
  providers + 4 nano-robots actifs
- Helper `build_mission_b_request(query, llm_ids, sources)`
- Helper `gemini_gpt_endpoints_alive()` (équivalent multi-provider de check_live_env)
- **Nouveau test** : `live_mission_b_full_3llm_consensus_with_pre_chewing` (~150 LOC)

### 3.4 `tests/workflow_mission_b.rs` (PATCH +3 tests offline ~200 LOC)

Analogues aux tests offline G3 de S4 :
- `mission_b_chewing_metrics_filled_with_mocks`
- `mission_b_emits_pre_chewing_metrics_event`
- `mission_b_chewing_metrics_serializable_through_response`

### 3.5 Fixture réutilisée

Pas de nouvelle fixture en S5 ! On réutilise `03_factual_compressible.json` :
- query 547 chars → compressor déclenché
- 2 sources Wikipédia
- expected_keyword "paris" → consensus 3-LLM

(Note : l'extension serait `04_factual_with_numbers.json` pour S6 router tuning, pas pour S5.)

### 3.6 `S5_LIVE_VALIDATION.template.md` (NOUVEAU)

Template à remplir post-run (modèle S1-S4).

### 3.7 `docs/architecture/...` (CE FICHIER)

Référence durable du contrat S5.

---

## 4. Spec détaillée du test `live_mission_b_full_3llm_consensus_with_pre_chewing`

### 4.1 Pré-conditions

| Pré-condition | Vérification | Action si fail |
|---|---|---|
| `EFE_LIVE_LLM=1` | env | `[skip]` propre |
| `EFE_LIVE_SLM=1` | env | `[skip]` propre |
| `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY` + `OPENAI_API_KEY` | tous présents | `[skip]` ou panic explicite |
| analyzer :8090 + compressor :8092 + verifier :8093 | HTTP /health × 3 | `[skip]` propre |

### 4.2 Assertions D1-D25

| # | Assertion | Justification |
|---|---|---|
| D1 | `final_text` non vide | LLM gagnant a produit du texte |
| D2 | `final_text.to_lowercase().contains("paris")` | Convergence factuelle |
| D3 | `winner_llm ∈ {claude_…, gemini_…, gpt_…}` | C'est un des 3 vrais providers |
| D4 | `per_llm_scores.len() == 3` | Les 3 ont participé au vote |
| D5 | **Tous les scores per-LLM ≥ 1.0** | Tous ont produit un ballot valide |
| D6 | `total_cost_cents ≤ 10` | Cap large (3 LLMs × ~3¢ max) |
| D7 | `total_cost_cents >= 1` | Au moins 1¢ (les 3 LLMs n'étaient pas mock) |
| D8 | `latency_ms < 30 000` | Cap deadline |
| D9 | `audit_chain_hash.len() == 64` | BLAKE3 hex |
| D10 | `verdicts.len() >= 1` | Verifier a tourné |
| D11 | `verdicts[0].verdict == "SUPPORTED"` | Fact-check correct |
| D12 | `verdicts[0].similarity >= 0.3` | Cosine match |
| D13 | `chewing_metrics.is_some()` | **S5 NEW : MissionBResponse extension OK** |
| D14 | `input_tokens_raw >= 100` | Query longue |
| D15 | `input_tokens_compressed < raw` | Compressor a réduit |
| D16 | **`compression_ratio <= 0.7`** | Cible savings (déjà 0.152 en S4) |
| D17 | `savings_tokens >= 30` | Économies tangibles |
| D18 | `compressor_latency_ms > 0 && < 10_000` | A tourné |
| D19 | `llm_latency_ms < 15_000` | max des 3 LLMs |
| D20 | `selected_llm == winner_llm` | Cohérence champ Response |
| D21 | tags contient `"VOTE_RESOLVED"` | Vote émis |
| D22 | tags contient `"PRE_CHEWING_METRICS"` | S5 émission |
| D23 | tags `"ROBOT_FINISHED"` au moins 2× (analyzer + compressor + verifier) | 3 robots actifs |
| D24 | wall_clock < 35_000 | Sanity globale |
| D25 | **Vérification multiplicatif** : `savings_tokens × 3 >= 60` (savings cumulés sur les 3 LLMs) | **Démontre la multiplication économique** |

### 4.3 Sorties console attendues

```text
=== Mission B 3-LLM consensus + pré-mâchage MUTUALISÉ ===
  query_raw    : "Je suis en train…" (547 chars, ~132 tokens)
  3 LLMs       : claude_claude-sonnet-4-6 + gemini_gemini-2.5-flash + gpt_gpt-4o
  4 robots     : analyzer + compressor + verifier + router (None pour reformulator/filter)

  ── Phase 1 : Pré-mâchage MUTUALISÉ ──
    analyzer    : ~4000 ms (20 entités → blackboard)
    compressor  : ~2000 ms (132 → 20 tokens, ratio=0.15, savings=112)
    ► Le savings de 112 tokens s'applique × 3 LLMs en aval ◄
    Savings cumulés : 112 × 3 = 336 tokens d'entrée économisés

  ── Phase 2 : Fan-out 3 LLMs parallèle ──
    claude    : ~3000 ms (32 tokens out, cost ~1¢)
    gemini    : ~1500 ms (28 tokens out, cost <1¢)
    gpt       : ~2200 ms (36 tokens out, cost ~1¢)
    Total :    parallel max ~3000 ms, cumulés out 96 tokens, cumulés cost ~3¢

  ── Phase 3 : Vote Symphony ──
    winner      : claude_claude-sonnet-4-6
    winner_score: 1.0
    per_llm_scores: {claude: 1.0, gemini: 1.0, gpt: 1.0} (consensus 3/3)

  ── Phase 4 : Verifier post-vote ──
    verifier   : ~3500 ms
    verdict    : SUPPORTED
    similarity : 0.85

  ── Mission B Response ──
    final_text       : "Paris est la capitale de la France."
    winner_llm       : claude_claude-sonnet-4-6
    total_cost_cents : 3
    latency_ms       : ~14_000 ms (analyzer + compressor + max(LLMs) + verifier)
    audit_hash       : abc…def (64 chars)
    chewing_metrics  : Some(ChewingMetrics { ratio=0.15, savings × 3 LLMs })
    events_seen      : ~45+ tags incluant VOTE_RESOLVED + PRE_CHEWING_METRICS

test live_mission_b_full_3llm_consensus_with_pre_chewing ... ok
test result: ok. 4 passed; 0 failed   (S2 + S3 + S4 + S5)
```

---

## 5. Plan d'implémentation chirurgical S5

8 sub-sprints de 20-40 min, committables indépendamment :

| # | Sprint | Durée | Livrable | Critère |
|---|---|---|---|---|
| **S5.1** | `MissionBResponse.chewing_metrics: Option<...>` | 10 min | request.rs +5 LOC | compile clean |
| **S5.2** | `mission_b.rs` instrumentation latences + métriques | 60 min | workflow ~60 LOC | tests offline existants passent |
| **S5.3** | `mission_b.rs` émission `PreChewingMetrics` event | 15 min | event émis avant SwarmFinished | bus tests passent |
| **S5.4** | 3 tests offline `mission_b_chewing_metrics_*` | 45 min | workflow_mission_b.rs +200 LOC | 3 nouveaux tests PASS |
| **S5.5** | Helper `make_coord_for_mission_b_pipeline` | 30 min | live_missions.rs +50 LOC | compile clean |
| **S5.6** | Test `live_mission_b_full_3llm_consensus_with_pre_chewing` | 60 min | live_missions.rs +200 LOC (25 assertions) | skip propre |
| **S5.7** | Compile + clippy + 414/414 PASS | 15 min | non-régression | tests verts |
| **S5.8** | Test live + S5_LIVE_VALIDATION.md + commit | 30 min | scellement | commit `feat(phase-3-r2-s5): …` |

**Total temps** : ~4 h 25 min cumulés + buffer = **~4-5 heures**.

---

## 6. Critères d'acceptation binaires S5

S5 est **livré** quand TOUS ces critères sont vrais :

- [ ] `MissionBResponse.chewing_metrics: Option<ChewingMetrics>` avec `#[serde(default)]`
- [ ] `run_mission_b` instrumenté (5 latences phase + agrégats fan-out)
- [ ] `StreamEvent::PreChewingMetrics` émis par `run_mission_b` avant `SwarmFinished`
- [ ] 3 nouveaux tests offline dans `workflow_mission_b.rs` (chewing_metrics_filled + emits_event + serializable)
- [ ] Helper `make_coord_for_mission_b_pipeline` dans `live_missions.rs`
- [ ] Test `live_mission_b_full_3llm_consensus_with_pre_chewing` avec 25 assertions D1-D25
- [ ] Skip propre × 4 cas (env vars, SLM, clés API)
- [ ] `cargo build --features all-providers --tests` clean
- [ ] `cargo clippy --tests --no-deps` 0 warning sur le code S5
- [ ] **0 régression** : 411 → 414 PASS (test S5 skip + 3 offline)
- [ ] Test live PASS avec env + 3 daemons UP + 3 clés API
- [ ] **25/25 assertions D1-D25** vertes
- [ ] **compression_ratio ≤ 0.7** observé
- [ ] **Vote unanime 3/3 sur "Paris"** observé
- [ ] **total_cost_cents ≤ 10** observé
- [ ] `S5_LIVE_VALIDATION.md` scellé
- [ ] Commit final `feat(phase-3-r2-s5): Mission B 3-LLM consensus + pré-mâchage mutualisé`

---

## 7. Hors-scope S5 (présents en S6+)

| Item | Sprint cible | Pourquoi pas S5 |
|---|---|---|
| Vote pondéré custom (`vote_weights` non uniform) | Phase 4+ | Pas critique pour démo économique |
| Router tuning Pareto par task_class | S6 | Mission B n'utilise pas le router (tous les LLMs sont appelés) |
| Quorum custom (au-delà du défaut N/2) | S6+ | Use case rare |
| Reformulator + filter | S5b ou Phase 4 | Toujours non utilisés dans workflows |
| Mission Speculative | S7 | drafter SLM + verifier LLM, pattern différent |
| Failure modes (1 LLM 429, vote dégradé 2/3) | S8 | Tests négatifs |
| 5+ fixtures variées | S9 | Savings report multi-cas |
| Tokenizer précis (tiktoken) | Phase 4 | Heuristique 1/4 suffit |

---

## 8. Risques connus & mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| 1 des 3 clés API expirée → 2/3 LLMs répondent | Moyenne | Vote dégradé (mais quorum 2 → toujours OK) | Test live skip si une clé manque |
| Gemini quota free-tier épuisé | Moyenne | 1/3 fail → vote sur 2 | Cap large `D4: scores.len() >= 2` (au lieu de 3) ? Non — exigeons 3 et on attend que quotas reset |
| Vote pas unanime (Claude dit "Paris", Gemini dit "Lutèce") | Faible | Vote 1-1-1 sans cluster majoritaire → fallback I5 | Query factuelle simple force consensus |
| Modif `run_mission_b` casse tests offline | Moyenne | régression | Validation S5.7 OBLIGATOIRE entre S5.2 et S5.5 |
| Latence parallèle dépasse 30 s | Faible | D8 fail | Cap budget MissionBRequest.max_total_latency assez large |
| `chewing_metrics.llm_latency_ms` non cohérent (max vs somme) | Moyenne | confusion sémantique | Documenter clairement : `max()` en Mission B (parallèle), `direct` en Mission A |

---

## 9. Vérification post-livraison

Une fois S5 commit, on doit pouvoir reproduire :

```bash
$ ./scripts/slm/start.sh && ./scripts/slm/health.sh    # 4/4 ● UP
$ ./scripts/slm/check_live_env.sh                       # 3 clés API ✓
$ EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 \
    cargo test -p efe-coordinator --features all-providers \
      --test live_missions -- --nocapture

running 4 tests
test live_mission_a_naked_no_slm_just_claude ... ok           (S2)
test live_mission_a_with_analyzer_verifier ... ok             (S3)
test live_mission_a_full_pre_chewing_pipeline ... ok          (S4)
=== Mission B 3-LLM consensus + pré-mâchage MUTUALISÉ ===     (S5 NEW)
…
test live_mission_b_full_3llm_consensus_with_pre_chewing ... ok

test result: ok. 4 passed; 0 failed
```

---

## 10. Lien avec les sprints suivants

### Pour S6 (Router tuning par task_class)
- En Mission A le router choisit 1 LLM. En Mission B, tous sont appelés.
- S6 utilisera la **Mission A pipeline** pour tester le routing (cas "code → Claude", "factual → Gemini Flash", etc.)

### Pour S7 (Mission Speculative)
- Pattern différent : drafter SLM + verifier LLM (pas multi-LLM)
- Réutilisera `ChewingMetrics` adapté pour speculation

### Pour S9 (Chewing savings report)
- 5 fixtures × **2 modes** (baseline LLM-only vs full pre-chewing) × **2 missions** (A et B)
- Tableau récap : "savings × N LLMs en Mission B" sera la métrique-phare

---

## 11. Conclusion

S5 démontre que **le pré-mâchage SLM est asymptotiquement N× plus rentable
quand N LLMs sont appelés en parallèle** (Mission B vs Mission A).

Concrètement :
- **S4** : 1 LLM → savings 85 % tokens → coût LLM divisé par 3
- **S5** : 3 LLMs en parallèle → savings 85 % × 3 = **336 tokens cumulés économisés** → coût LLM réduit de ~60 % sur la mission tri-LLM

C'est l'argument économique multiplié qui justifie EFE comme **couche
d'orchestration multi-LLM** (vs un appel direct à 3 LLMs séparés depuis
le client).

**Prochaine action concrète** : démarrer S5.1 (ajouter `chewing_metrics` à
`MissionBResponse`).
