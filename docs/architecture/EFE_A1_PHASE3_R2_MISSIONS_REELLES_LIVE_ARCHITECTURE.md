# EFE Phase 3 R2 — Missions Réelles Live : Architecture Chirurgicale

**Statut** : Spec ready-to-implement
**Date** : 2026-05-27
**Précédence** : R1a + R1b validés live 18/18 (snapshot `crates/efe-coordinator/R1b_LIVE_VALIDATION.md`)
**Prochaine étape codable** : Sprint S1 ci-dessous

---

## 0. Le but premier — Prémâcher le travail des LLMs

> **Sans pré-mâchage, R2 n'a pas de raison d'exister.**

Un gros LLM (Claude Sonnet 4.6, Gemini 2.5 Flash, GPT-4o) coûte 100× à 1000× plus
cher au million de tokens qu'un SLM local (SmolLM2-360M, Qwen3-0.6B, Llama-3.2-1B,
Phi-4-mini) tournant sur llama-server. Le but de Phase 1 est de **transformer
un input utilisateur brut en un input optimisé pour le gros LLM** :

1. **Analyzer** (SmolLM2-360M, port 8090) extrait l'intent, les entités, la langue → input devient structuré
2. **Filter** (bge-m3 embeddings, ONNX local) rank les contextes candidats par pertinence → on garde seulement le signal
3. **Router** (heuristique Pareto + bge-m3) choisit le gros LLM optimal pour la tâche (cost/quality/latency)
4. **Reformulator** (Qwen3-0.6B, port 8091) réécrit la requête en canonique (sans dialecte, sans ambiguïté)
5. **Compressor** (Llama-3.2-1B, port 8092) compresse à 30-50 % des tokens en préservant le sens
6. **[GROS LLM]** (Claude/Gemini/GPT) génère sur input pré-mâché
7. **Verifier** (Phi-4-mini, port 8093) fact-check + détecte hallucinations

**Économie cible R2** :
- **−40 à −60 % tokens d'entrée** envoyés au gros LLM (effet Compressor + Reformulator)
- **−30 à −50 % coût $/requête** (effet Router qui pick le bon LLM pour la tâche)
- **+1 ordre de qualité** sur l'output final (effet Verifier filtre les hallucinations)
- **Latence stable < 5 s** pour query simple, < 15 s pour query complexe (effet parallélisme SLM)

**Sans pré-mâchage**, EFE = juste un proxy LLM bête. **Avec pré-mâchage**, EFE
= une couche d'optimisation économique et qualitative qui transforme un input
brut en or pré-digéré pour les LLMs gros.

---

## 1. Audit pré-R2 — État du code existant

### Ce qui est déjà en place (vérifié dans le repo)

| Composant | Statut | Référence |
|---|---|---|
| 6 nano-robot clients Phase 1 (trait `NanoRobotClient`) | ✅ Codés | `crates/efe-nano-app-coordinator/src/clients/*.rs` |
| Trait LLM provider (3 vrais : Claude, Gemini, GPT) | ✅ Live-validé | `crates/efe-coordinator/src/llm/providers/{anthropic,gemini,openai}.rs` |
| `run_mission_a` / `run_mission_b` / `run_speculative` | ✅ Codés | `crates/efe-coordinator/src/workflows/*.rs` |
| `CoordinatorBuilder` accepte vrais clients + vrais providers | ✅ | `crates/efe-coordinator/src/coordinator.rs:114-191` |
| `DagExecutor` + `DagNode` + `ParentOutputs` | ✅ Codés | `crates/efe-coordinator/src/dag/*.rs` |
| Audit chain HMAC + Blackboard phéromones + EventBus SSE | ✅ Codés | `audit.rs`, `blackboard/`, `events/` |
| `MultiLlmBridge` + Symphony vote | ✅ Codés | `llm/bridge.rs`, `vote/engine.rs` |

### Ce qui manque pour R2 (gap analysis)

| Gap | Sévérité | Solution R2 |
|---|---|---|
| **G1** — Aucun test live qui exécute une mission complète avec vrai llama-server + vrais LLMs | 🔴 critique | `tests/live_missions.rs` env-gated `EFE_LIVE_LLM=1` + `EFE_LIVE_SLM=1` |
| **G2** — Aucun script de bootstrap pour lancer les 4 llama-server (ports 8090-8093) | 🔴 critique | `scripts/start_llama_servers.sh` + assets/models téléchargés |
| **G3** — Pas de fixtures de requêtes utilisateur réalistes pour valider missions | 🟡 important | `tests/fixtures/missions/*.json` (8 queries diverses) |
| **G4** — Pas de mesure objective du gain de pré-mâchage (tokens avant/après) | 🟡 important | Module `metrics::ChewingMetrics` exposé dans events |
| **G5** — Le `RouterClient` est codé mais sa stratégie Pareto n'a pas été tunée sur les 3 vrais LLMs | 🟡 important | Sprint S6 : table de scoring (cost/latency/quality par LLM par task_class) |
| **G6** — Pas d'endpoint HTTP/SSE qui expose les missions à un client externe | 🟢 nice-to-have | Out-of-scope R2 (Phase 4 frontend) |
| **G7** — Le `live_providers.rs` ne couvre que les providers nus, pas les missions | 🟢 nice-to-have | Couvert par G1 |
| **G8** — Heartbeat + Audit chain non testés en mission complète sous charge | 🟢 nice-to-have | Sprint S8 stress test optionnel |

**Verdict** : 80 % du code est déjà écrit. R2 = **opération de branchement et de validation**, pas une réécriture. Ce qui manque est de l'infrastructure de test + tuning du router + assets de modèles.

---

## 2. Architecture globale R2

### 2.1 Schéma en couches

```text
┌──────────────────────────────────────────────────────────────────────────┐
│  Couche application — utilisateur final (humain ou agent IA)             │
│  POST /mission/a   POST /mission/b   POST /mission/speculative           │
│  ←── SSE stream (StreamEvent v1.0.0)                                     │
└──────────────────────────────┬───────────────────────────────────────────┘
                               ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  Couche Coordinator (efe-coordinator, R2 = wiring complet)               │
│  • workflows::run_mission_{a,b,speculative}                              │
│  • DagExecutor : Kahn topo + tokio::Semaphore(max_concurrency=8)         │
│  • PheromoneBlackboard 256 KB : phéromones partagées entre nodes         │
│  • EventBus : InMemoryBus (default) ou NatsBus (feature `nats-bus`)      │
│  • HmacAuditChain : 1 entry par invocation de robot (invariant I1)       │
└────────────┬───────────────────────────────────────────────┬─────────────┘
             ▼                                                ▼
┌────────────────────────────────────┐    ┌─────────────────────────────────┐
│  Phase 1 — nano-robots (SLM)       │    │  Phase 3 R1 — providers (LLM)   │
│  Pré-mâchage + verifier            │    │  Gros LLMs                      │
│                                    │    │                                 │
│  • analyzer    :8090 (SmolLM2)     │    │  • Claude Sonnet 4.6  (https)   │
│  • reformulator:8091 (Qwen3-0.6B)  │    │  • Gemini 2.5 Flash   (https)   │
│  • compressor  :8092 (Llama-3.2-1B)│    │  • GPT-4o             (https)   │
│  • verifier    :8093 (Phi-4-mini)  │    │                                 │
│  • filter      (ONNX bge-m3 local) │    │  Auth : ApiKey wrapper redacted │
│  • router      (heuristique local) │    │  Retry + circuit breaker        │
│                                    │    │  Rate-limit observability       │
│  HTTP llama-server b9297+          │    │  PII redactor I15               │
│  Grammar GBNF enforcement          │    │                                 │
└────────────────────────────────────┘    └─────────────────────────────────┘
```

### 2.2 Topologie DAG par mission

#### Mission A — mono-LLM avec pré-mâchage complet

```text
                    ┌───────────────┐
                    │  user_query   │  (root)
                    └───────┬───────┘
                            ▼
                    ┌───────────────┐
                    │  analyzer     │  (SLM :8090, ~150 ms)
                    │  intent+ent   │
                    └───────┬───────┘
                            ▼
        ┌─────────────┬─────────────┬─────────────┐
        ▼             ▼             ▼             │  ← fan-out parallèle SLM
┌────────────┐ ┌────────────┐ ┌────────────┐      │
│  filter    │ │  router    │ │ reformulator│     │   (~80, ~120, ~200 ms)
│  bge-m3    │ │  Pareto    │ │  Qwen3      │     │
└──────┬─────┘ └──────┬─────┘ └──────┬──────┘     │
       └──────────────┴──────────────┘            │
                      ▼                            │
              ┌───────────────┐                    │
              │  compressor   │  (SLM :8092, ~250 ms)
              │  Llama-3.2-1B │
              └───────┬───────┘
                      ▼
              ┌───────────────┐
              │  llm_call     │  (gros LLM choisi par router, ~1-2 s)
              │  Claude/Gemini│
              │  /GPT         │
              └───────┬───────┘
                      ▼
              ┌───────────────┐
              │  verifier     │  (SLM :8093, ~300 ms)
              │  Phi-4-mini   │
              └───────┬───────┘
                      ▼
                   final_text
```

Latence totale estimée : **~2.5 s à 4 s** pour query simple.
Coût estimé : **~1-3 ¢ pour Claude/GPT, < 1 ¢ pour Gemini Flash**.

#### Mission B — multi-LLM avec pré-mâchage partagé

```text
                user_query
                    │
                    ▼
                 analyzer ─► filter ┐
                    │       reform. ├── (fan-out SLM)
                    │       router  ┘
                    ▼
                 compressor   ◄── pré-mâchage UNE seule fois
                    │
        ┌───────────┼───────────┐
        ▼           ▼           ▼   ← fan-out 3 gros LLMs en parallèle
   Claude Sonnet  Gemini Flash  GPT-4o
        │           │           │
        └───────────┼───────────┘
                    ▼
              VoteEngine Symphony   ← convergence sémantique pondérée
                    │
                    ▼
                 verifier global    ← fact-check sur winning text
                    │
                    ▼
                final_text + audit_chain
```

Latence totale estimée : **~3 s à 5 s** (parallélisme bornée par LLM le plus lent).
Coût estimé : **~3-9 ¢** (= 1× pré-mâchage + 3× LLM call).
**Économie clé** : compresseur appliqué UNE fois → 3 × moins de tokens d'entrée pour les LLMs gros qu'une approche naïve.

#### Mission Speculative — drafter SLM + verifier LLM

```text
              user_query
                  │
       ┌──────────┴──────────┐
       ▼                     ▼   ← démarrent en PARALLÈLE
  drafter (SLM, local,    big LLM call (~1-2 s)
  sub-second)             "complete: <user_query>"
       │                     │
       ▼                     │
   draft text                │
       │                     │
       └──────────┬──────────┘
                  ▼
            speculate engine
            jaccard(draft, llm_text) ≥ 0.5 ?
                  │
       ┌──────────┴──────────┐
       ▼                     ▼
    accepté:              rejeté:
    final = draft         final = llm_text
    (latence_saved)       (full latence)
                  │
                  ▼
              verifier (SLM :8093, ~300 ms)
                  │
                  ▼
             final_text
```

**Optimisation R2** : si le drafter SLM est suffisamment confiant (verifier
local agrees), on **peut court-circuiter** le gros LLM → coût ~0.01 ¢ au lieu
de 1-3 ¢. Cible : 30-50 % des queries simples acceptées par drafter.

---

## 3. Plan d'implémentation R2 — Sprints chirurgicaux

Chaque sprint est **petit, mesurable et committable indépendamment**. Aucun
sprint ne touche > 5 fichiers source. Chaque sprint produit des tests qui
verrouillent le comportement (offline d'abord, live ensuite quand applicable).

### S1 — Bootstrap llama-server local (gap G2)

**But** : lancer les 4 instances llama-server (ports 8090-8093) avec les
modèles requis, en une commande.

**Livrables** :
- `scripts/start_llama_servers.sh` — bash qui démarre 4 daemons llama-server avec les bons modèles, grammaires et ports.
- `scripts/stop_llama_servers.sh` — kill propre.
- `scripts/health_check_slm.sh` — ping `/health` des 4 endpoints, retour 0 si tous OK.
- `crates/efe-nano-app-coordinator/MODELS_DOWNLOAD.md` — instructions pour télécharger les 4 modèles GGUF (HuggingFace URLs précises).
- Optionnel : `docker-compose.slm.yml` pour run dans conteneurs (Phase 4).

**Tests** : `scripts/test_bootstrap.sh` qui démarre, ping `/health`, kill. CI ignore (besoin local).

**Critère d'acceptation S1** :
```bash
$ ./scripts/start_llama_servers.sh && ./scripts/health_check_slm.sh && echo "✅ 4/4 SLM up"
✅ 4/4 SLM up
```

**Estimation** : 2-3 heures (téléchargements modèles ~5-10 GB cumulés).

---

### S2 — Test live Mission A nu (1 query → 1 LLM via Coordinator)

**But** : prouver que la chaîne **vraie** fonctionne bout-en-bout avec
**aucun nano-robot** activé (clients tous `None`), juste pour valider le
branchement Coordinator ↔ LlmProvider via `run_mission_a`.

**Livrables** :
- `crates/efe-coordinator/tests/live_missions.rs` (nouveau fichier).
- `tests/fixtures/missions/factual_simple.json` — `{"query": "Quelle est la capitale de la France ?", "expected_keyword": "Paris"}`.
- Premier test :
  ```rust
  #[tokio::test]
  async fn live_mission_a_naked_no_slm_just_claude() { ... }
  ```
- Le test instancie un `Coordinator` avec `CoordinatorClients::empty()` (aucun nano-robot), 1 provider Claude, exécute `run_mission_a`, vérifie que `MissionAResponse.final_text` contient "Paris".

**Critère d'acceptation S2** : 1 test live PASS en `EFE_LIVE_LLM=1`, coût < 2 ¢.

**Estimation** : 2 heures.

---

### S3 — Mission A avec analyzer + verifier (sans compressor/router/reformulator)

**But** : ajouter progressivement les nano-robots, en commençant par les 2
qui ont le plus de valeur sans risque (analyzer pour structurer, verifier
pour valider l'output).

**Livrables** :
- Test `live_mission_a_with_analyzer_verifier`.
- Si llama-server :8090 et :8093 sont down → test `[skip]` propre.
- Vérifie : `swarm_finished` event reçu, audit_chain_hash non-vide, verifier_confidence ≥ 0.5.

**Critère d'acceptation S3** : 2 tests live PASS (S2 + S3).

**Estimation** : 3 heures (premier test qui exerce les vrais SLM).

---

### S4 — Mission A complète (analyzer + reformulator + compressor + verifier)

**But** : exécuter le pipeline pré-mâchage **complet** sur 1 query, mesurer
les tokens avant/après compression, valider que `final_text` est cohérent.

**Livrables** :
- Test `live_mission_a_full_pre_chewing_pipeline`.
- Module `metrics::ChewingMetrics` (gap G4) :
  ```rust
  pub struct ChewingMetrics {
      pub input_tokens_raw: u32,
      pub input_tokens_after_compressor: u32,
      pub compression_ratio: f32,
      pub slm_total_latency_ms: u64,
      pub llm_latency_ms: u64,
      pub total_cost_cents: u32,
  }
  ```
- Émission d'un `StreamEvent::PreChewingMetrics(metrics)` à la fin du DAG.
- Assertion : `compression_ratio ≤ 0.7` (30 % de réduction au minimum).

**Critère d'acceptation S4** : pipeline E2E PASS + métriques visibles dans stream events.

**Estimation** : 4 heures.

---

### S5 — Mission B complète (3-LLM consensus avec pré-mâchage partagé)

**But** : démontrer que le pré-mâchage est **mutualisé** sur les 3 LLMs en
fan-out → économie multiplicative.

**Livrables** :
- Test `live_mission_b_full_3llm_vote`.
- Vérifie :
  - 3 réponses des 3 vrais providers.
  - Symphony vote retourne un winning_text.
  - `swarm_finished.total_cost_cents ≤ 10` (≤ 10 ¢ pour la full mission).
  - Compresseur exécuté **1 seule fois** (pas 3).

**Critère d'acceptation S5** : 4 tests live PASS, vote consensus sur fixture "Paris" 3/3.

**Estimation** : 3 heures.

---

### S6 — Tuning du Router pour les 3 vrais LLMs (gap G5)

**But** : le router doit choisir intelligemment Claude vs Gemini vs GPT
selon la tâche.

**Livrables** :
- Table de scoring statique `crates/efe-coordinator/src/llm/router_strategy.rs` :
  ```rust
  // cost_score (0-100, lower=cheaper), quality_score (0-100), latency_score (0-100, lower=faster)
  // par task_class
  pub static ROUTING_TABLE: &[(TaskClass, &str, RoutingScores)] = &[
      (TaskClass::FactualQA,      "gemini-2.5-flash",   RoutingScores { cost: 2,   quality: 75, latency: 5 }),
      (TaskClass::FactualQA,      "gpt-4o",             RoutingScores { cost: 40,  quality: 88, latency: 12 }),
      (TaskClass::FactualQA,      "claude-sonnet-4-6",  RoutingScores { cost: 50,  quality: 92, latency: 15 }),
      (TaskClass::CodeGen,        "claude-sonnet-4-6",  RoutingScores { cost: 50,  quality: 95, latency: 15 }),
      (TaskClass::CodeGen,        "gpt-4o",             RoutingScores { cost: 40,  quality: 88, latency: 12 }),
      (TaskClass::LongReasoning,  "claude-sonnet-4-6",  RoutingScores { cost: 50,  quality: 96, latency: 25 }),
      (TaskClass::SummaryShort,   "gemini-2.5-flash",   RoutingScores { cost: 2,   quality: 80, latency: 5 }),
      // ... ~15 entrées au total
  ];
  ```
- Fonction `pick_best(task_class, budget_priority, latency_priority) -> ProviderId`.
- Test offline : pour 8 task_class × 3 priorités → vérifier choix attendu.

**Critère d'acceptation S6** : 24 tests offline PASS + 2 tests live `live_router_picks_gemini_for_factual` et `live_router_picks_claude_for_code`.

**Estimation** : 4 heures.

---

### S7 — Mission Speculative complète (drafter SLM + verifier LLM)

**But** : valider que le pattern `Speculator` court-circuite le gros LLM
quand le drafter SLM est confiant.

**Livrables** :
- Test `live_mission_speculative_simple_factual` :
  drafter = SmolLM2 :8090, verifier = Claude → query "Capital of France?" → draft "Paris" → Claude verify → accepté → final = "Paris", `latency_saved > 0`.
- Test `live_mission_speculative_complex_rejected` :
  drafter SLM fait une erreur, Claude rejette, final = Claude output.

**Critère d'acceptation S7** : 2 tests live PASS.

**Estimation** : 3 heures.

---

### S8 — Robustness : failure modes (gap G8)

**But** : valider que le DAG continue (avec dégradation gracieuse) si un nœud crash.

**Livrables** :
- Test `live_mission_a_with_slm_8092_down` : compressor offline → mission continue avec compressor skip (bypass), input non compressé envoyé au LLM, log warning.
- Test `live_mission_a_with_llm_429` : LLM 429 → retry exponentiel, succès au 2ᵉ ou 3ᵉ essai.
- Test `live_mission_b_with_1_of_3_llm_dead` : 1 LLM échoue → vote sur 2 restants.

**Critère d'acceptation S8** : 3 failure-mode tests PASS, 0 panic.

**Estimation** : 4 heures.

---

### S9 — Mesure objective du gain pré-mâchage (gap G4 finalisé)

**But** : produire un rapport reproductible chiffrant l'économie.

**Livrables** :
- Test `live_mission_a_chewing_savings_report` qui exécute 5 fixtures de queries variées :
  - factual_simple, factual_complex, code_gen, summary_long, multi_step_reasoning.
- Pour chaque : compare baseline (LLM-only, sans nano-robots) vs full pre-chew.
- Génère `crates/efe-coordinator/CHEWING_SAVINGS_REPORT.md` avec tableau comparatif.

**Cible R2** :
| Métrique | Baseline (LLM-only) | Avec pré-mâchage | Gain |
|---|---|---|---|
| Tokens input moyens | 100 | 45 | **−55 %** |
| Coût moyen ¢/query | 6 | 2.7 | **−55 %** |
| Latence moyenne ms | 1500 | 2200 | **+47 %** (compromis) |
| Hallucinations détectées | 0/5 | 2/5 | **+infinitif** (filtres ajoutés) |

Acceptable : latence en hausse modérée (le pré-mâchage prend du temps), mais
coût et qualité explosent positivement.

**Critère d'acceptation S9** : rapport généré, économie ≥ 30 % sur tokens.

**Estimation** : 5 heures.

---

### S10 — Documentation R2 + commit final

**Livrables** :
- `crates/efe-coordinator/R2_LIVE_VALIDATION.md` — snapshot des tests R2 PASS.
- Mise à jour `CHANGELOG.md` section R2.
- Mise à jour `lib.rs` doc avec exemple end-to-end mission.
- Commit final `feat(phase-3-r2): missions réelles live — pré-mâchage validé end-to-end`.

**Estimation** : 2 heures.

---

## 4. Détail technique — DAG Mission A en code Rust

Voici le squelette du DAG Mission A R2 (concret, prêt à coder dans `workflows/mission_a.rs`) :

```rust
pub async fn run_mission_a(
    coord: &Coordinator,
    req: MissionARequest,
) -> CoordResult<MissionAResponse> {
    let swarm_id = uuid::Uuid::new_v4();
    coord.bus.publish(StreamEvent::SwarmStarted { swarm_id, ... }).await;

    let mut nodes = Vec::new();

    // Node 0 — root : user query
    nodes.push(DagNode::root("user_query", {
        let query = req.query.clone();
        move || Box::pin(async move { Ok(json!({ "text": query })) })
    }));

    // Node 1 — analyzer (optionnel, skip si pas instancié)
    if let Some(analyzer) = coord.clients.analyzer.clone() {
        nodes.push(DagNode::with_parents_and_inputs("analyzer", vec!["user_query".into()], move |parents| {
            let analyzer = analyzer.clone();
            Box::pin(async move {
                let text = parents["user_query"]["text"].as_str().unwrap().to_string();
                let out = analyzer.invoke(AnalyzerInput { text, max_entities: 8, source_agent: "mission_a".into() }, &ctx).await?;
                Ok(serde_json::to_value(&out)?)
            })
        }).with_timeout(Duration::from_secs(5)));
    }

    // Nodes 2a/2b/2c — filter ∥ router ∥ reformulator (fan-out parallèle)
    if let Some(filter) = coord.clients.filter.clone() {
        nodes.push(DagNode::with_parents_and_inputs("filter", vec!["analyzer".into()], ...));
    }
    if let Some(router) = coord.clients.router.clone() {
        nodes.push(DagNode::with_parents_and_inputs("router", vec!["analyzer".into()], ...));
    }
    if let Some(reformulator) = coord.clients.reformulator.clone() {
        nodes.push(DagNode::with_parents_and_inputs("reformulator", vec!["analyzer".into()], ...));
    }

    // Node 3 — compressor (dépend des 3 précédents)
    if let Some(compressor) = coord.clients.compressor.clone() {
        nodes.push(DagNode::with_parents_and_inputs(
            "compressor",
            vec!["filter".into(), "router".into(), "reformulator".into()],
            ...
        ));
    }

    // Node 4 — llm_call (le pivot de la mission)
    nodes.push(DagNode::with_parents_and_inputs("llm_call", vec!["compressor".into(), "router".into()], move |parents| {
        let bridge = coord.bridge.clone();
        Box::pin(async move {
            // Lit le compressed prompt + router decision
            let compressed = parents["compressor"]["summary"].as_str().unwrap().to_string();
            let chosen_llm: ProviderId = parents["router"]["chosen_llm"].as_str().unwrap().to_string();
            let provider = bridge.registry().get(&chosen_llm).ok_or(CoordError::LlmProvider { ... })?;
            let resp = provider.call(LlmCall { prompt: compressed, max_tokens: req.max_tokens, ..Default::default() }).await?;
            Ok(serde_json::to_value(&resp)?)
        })
    }).with_timeout(Duration::from_secs(60)));

    // Node 5 — verifier (optionnel)
    if let Some(verifier) = coord.clients.verifier.clone() {
        nodes.push(DagNode::with_parents_and_inputs("verifier", vec!["llm_call".into()], ...));
    }

    // Exécute le DAG
    let report = coord.dag.execute(nodes).await;
    if report.any_failure {
        return Err(CoordError::Workflow(format!("DAG failed: {:?}", report.errors)));
    }

    // Récupère le final_text depuis verifier ou llm_call
    let final_text = report.outputs.get("verifier")
        .or_else(|| report.outputs.get("llm_call"))
        .and_then(|v| v["text"].as_str())
        .unwrap_or("")
        .to_string();

    coord.bus.publish(StreamEvent::SwarmFinished { swarm_id, final_text: final_text.clone(), ... }).await;

    Ok(MissionAResponse {
        final_text,
        llm_used: ...,
        latency_ms: ...,
        audit_chain_hash: ...,
    })
}
```

**Note clé** : `coord.clients.*` est `Option<Arc<...>>` → chaque nœud SLM est
**optionnellement ajouté au DAG**. Si l'utilisateur fait `CoordinatorClients::empty()`,
on saute directement à `llm_call` (mode "naked"). Cela permet de tester
incrémentalement (S2 naked, S3 partiel, S4 full).

---

## 5. Tests live R2 — Fichier `tests/live_missions.rs`

### 5.1 Pré-requis env

```bash
# Phase 3 LLMs (mêmes qu'en R1b)
export ANTHROPIC_API_KEY='sk-ant-...'
export GOOGLE_AI_KEY='AIza...'
export OPENAI_API_KEY='sk-...'

# Phase 1 SLMs (nouveau R2)
export EFE_LIVE_SLM=1   # active les tests qui dépendent de llama-server
export EFE_LIVE_LLM=1   # active les tests qui dépendent des providers

# Bootstrap SLMs en local
./scripts/start_llama_servers.sh   # démarre 4 daemons :8090-8093
```

### 5.2 Liste des 12 tests R2 cibles

| # | Test | Sprint | Composants exercés | Coût |
|---|---|---|---|---|
| 1 | `live_mission_a_naked_no_slm_just_claude` | S2 | Coord + Claude | 1-2 ¢ |
| 2 | `live_mission_a_with_analyzer_verifier` | S3 | Coord + analyzer + verifier + Claude | 2 ¢ |
| 3 | `live_mission_a_full_pre_chewing_pipeline` | S4 | Coord + 6 robots + Claude | 2-3 ¢ |
| 4 | `live_mission_a_with_gemini_route` | S6 | Router pick → Gemini | < 1 ¢ |
| 5 | `live_mission_a_with_gpt_route` | S6 | Router pick → GPT | 2 ¢ |
| 6 | `live_mission_b_full_3llm_vote` | S5 | Coord + pré-mâchage + 3 LLMs + vote | 3-9 ¢ |
| 7 | `live_mission_speculative_simple_factual` | S7 | Drafter SLM + Claude verifier | 1 ¢ |
| 8 | `live_mission_speculative_complex_rejected` | S7 | Drafter rejeté → fallback LLM | 2 ¢ |
| 9 | `live_mission_a_with_slm_8092_down` | S8 | Compressor down → graceful skip | 2 ¢ |
| 10 | `live_mission_a_with_llm_429_retry` | S8 | LLM 429 → retry → success | 2 ¢ |
| 11 | `live_mission_b_with_1_of_3_llm_dead` | S8 | Vote dégradé 2/3 | 2-6 ¢ |
| 12 | `live_mission_a_chewing_savings_report` | S9 | 5 queries × 2 modes → rapport tokens | 10-20 ¢ |

**Coût total R2 validation complet** : ~30-50 ¢ + temps SLM gratuit.
**Durée totale** : ~5 minutes pour 12 tests séquentiels.

### 5.3 Commande de validation finale R2

```bash
EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 \
  cargo test -p efe-coordinator --features all-providers \
  --test live_missions -- --test-threads=1 --nocapture
```

Cible : `test result: ok. 12 passed; 0 failed`.

---

## 6. Fixtures de requêtes utilisateur (gap G3)

`crates/efe-coordinator/tests/fixtures/missions/*.json` — 8 fixtures représentatives :

| Fichier | task_class | Query | Expected |
|---|---|---|---|
| `01_factual_simple.json` | FactualQA | "Quelle est la capitale de la France ?" | contient "Paris" |
| `02_factual_with_numbers.json` | FactualQA | "Quelle est la vitesse de la lumière ?" | contient "299" |
| `03_code_gen_rust.json` | CodeGen | "Écris une fonction Rust qui additionne deux i32." | contient "fn", "i32", "+" |
| `04_summary_short.json` | SummaryShort | "Résume en 1 phrase : <paragraphe de 200 mots>" | < 30 mots |
| `05_long_reasoning.json` | LongReasoning | "Explique pas-à-pas pourquoi 17 est premier." | mentionne divisibilité |
| `06_multi_step.json` | LongReasoning | "Si A>B et B>C, A>C est-il toujours vrai ? Justifie." | "transitivité" |
| `07_ambiguous.json` | Ambiguous | "Qu'est-ce que tu peux faire ?" | reformulator déclenché |
| `08_pii_heavy.json` | PiiSensitive | "Mon email est alice@example.com, peux-tu le saluer ?" | PII redactor déclenché, "alice" absent du log |

---

## 7. Observabilité R2 — Nouveaux StreamEvents

### 7.1 Événements ajoutés

```rust
// Dans crates/efe-coordinator/src/events/stream_event.rs (extension)
pub enum StreamEvent {
    // ... existants ...

    /// R2 — émis après l'analyzer
    AnalyzerCompleted {
        swarm_id: Uuid,
        intent: String,
        entities: Vec<String>,
        language: String,
        latency_ms: u64,
    },

    /// R2 — émis après le router (choix LLM)
    RouterChose {
        swarm_id: Uuid,
        chosen_llm: ProviderId,
        justification: String,
        alternatives_considered: Vec<ProviderId>,
    },

    /// R2 — émis après le compressor
    CompressorCompleted {
        swarm_id: Uuid,
        tokens_before: u32,
        tokens_after: u32,
        compression_ratio: f32,
        latency_ms: u64,
    },

    /// R2 — émis après le verifier
    VerifierCompleted {
        swarm_id: Uuid,
        verdict: VerifierVerdict,  // Confirmed / Uncertain / Hallucination
        confidence: f32,
        latency_ms: u64,
    },

    /// R2 — résumé global de pré-mâchage à la fin du DAG
    PreChewingMetrics {
        swarm_id: Uuid,
        metrics: ChewingMetrics,
    },
}
```

### 7.2 Backward compat

Les `StreamEvent` v1.0.0 sont stables. R2 ajoute de nouveaux variants → bump
implicite mineur (v1.1.0). Les clients SSE existants ignorent les variants
inconnus → pas de breaking change.

---

## 8. Sécurité et invariants R2

### 8.1 Invariants à vérifier en live

| Invariant | Description | Test live R2 |
|---|---|---|
| **I1** | 1 audit_chain entry par invocation de robot | `live_mission_a_audit_chain_complete` |
| **I2** | Blackboard ≤ 256 KB même sous DAG complet | `live_mission_a_blackboard_bounded` |
| **I3** | StreamEvent JSON roundtrip-stable | `live_mission_a_sse_events_roundtrip` |
| **I8** | DagExecutor borné à `max_concurrency` | `live_mission_b_concurrency_bound` |
| **I14** | ApiKey jamais loggué | `live_mission_a_logs_have_no_secrets` |
| **I15** | PII redactor appliqué avant envoi LLM | `live_mission_a_pii_redactor_active` |
| **I17** | CircuitBreaker 3-state opérationnel | déjà couvert en R1b |

### 8.2 PII redactor obligatoire en R2

Tout `Coordinator` R2 **doit** être instancié avec un PII redactor :
```rust
let redactor: PiiRedactor = Arc::new(|s| {
    // Patterns standards : email, CB, IBAN, numéro téléphone
    let s = email_regex.replace_all(s, "[REDACTED-EMAIL]");
    let s = cb_regex.replace_all(&s, "[REDACTED-CB]");
    // ...
    s.to_string()
});
let coord = Coordinator::builder()
    .clients(...)
    .registry(reg_with_redactor_propagated_to_each_provider)
    .build()?;
```

---

## 9. Hors-scope R2 (Phase 4 ou plus tard)

| Item | Pourquoi pas R2 |
|---|---|
| Endpoint HTTP/SSE public `POST /mission/*` | Frontend Phase 4 (preview lab) |
| Multi-tenant + per-tenant quotas | Phase 5+ enterprise |
| NATS JetStream prod bus | Phase 7 (feature `nats-bus` déjà codée, juste pas testée live) |
| Router ML-based (learned, pas heuristique) | Phase 8 RLHF |
| Cache prompt KV partagé entre missions | Phase 6 perf optimization |
| Fallback Ollama local pour gros LLM | R1c séparé (future provider) |
| Docker compose des 4 SLMs | Bonus S1, sinon Phase 4 |

---

## 10. Critères d'acceptation globaux R2

R2 est **livré et scellé** quand TOUS ces critères sont vrais :

- [ ] **12/12 tests live missions PASS** (`live_missions.rs`)
- [ ] **0 régression** sur la suite offline (`cargo test -p efe-coordinator --features all-providers` toujours 412+ PASS)
- [ ] **CHEWING_SAVINGS_REPORT.md** produit avec gain ≥ 30 % sur tokens d'entrée
- [ ] **R2_LIVE_VALIDATION.md** scellé avec snapshot exécution PASS
- [ ] **Mission A bench** : < 5 s p95 sur fixture factual_simple
- [ ] **Mission B bench** : < 7 s p95 sur fixture factual_simple
- [ ] **Mission Speculative bench** : < 2 s p95 quand draft accepté
- [ ] **0 PII fuite** dans les logs de tous les tests live
- [ ] **CHANGELOG.md** R2 section écrite

Si **un seul** critère échoue → R2 retourne en `[in_progress]` jusqu'à
résolution. **Pas de "presque livré"**, on est binaire.

---

## 11. Estimation effort total R2

| Sprint | Heures | Files touchés |
|---|---|---|
| S1 bootstrap llama-server | 3 | scripts/, docs/ |
| S2 mission_a naked | 2 | tests/live_missions.rs |
| S3 mission_a + analyzer + verifier | 3 | tests, fixtures/ |
| S4 mission_a full pipeline + ChewingMetrics | 4 | src/metrics.rs nouveau, events |
| S5 mission_b 3-LLM consensus | 3 | tests/live_missions.rs |
| S6 router tuning | 4 | src/llm/router_strategy.rs nouveau |
| S7 mission_speculative | 3 | tests/live_missions.rs |
| S8 failure modes | 4 | tests/live_missions.rs |
| S9 chewing savings report | 5 | reports/, tests |
| S10 doc + commit | 2 | CHANGELOG, R2_LIVE_VALIDATION.md |
| **Total** | **~33 heures** | ~15 fichiers source ; ~3000 LOC ajoutées |

**Sur 1 personne à plein temps** : 4-5 jours ouvrés.
**Coût $ validation** : < $1 cumulé (Claude + GPT crédits, Gemini gratuit en payant).
**Risque** : faible (80 % du code existe déjà). Le risque principal est S1
(téléchargement modèles + tuning hyperparamètres llama-server).

---

## 12. Risques et mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Modèles SLM trop gros pour Mac M-series | Moyenne | Bloque S1 | Fallback vers Q4 quantizations (plus petites) |
| Latence cumulée SLM > 2 s pour mission A | Faible | Performance dégradée | Parallélisation déjà en place (DAG fan-out) |
| Router heuristique mal calibré | Moyenne | Mauvais choix LLM | Tests S6 spécifiques + tunable via config |
| Compressor perd du sens critique | Moyenne | Output LLM dégradé | Verifier détecte → fallback prompt non-compressé |
| llama-server crash silencieux | Faible | Mission tombe | Health-check + circuit breaker côté NanoRobotClient |
| Coût Claude/GPT dépasse budget test | Faible | $$$ | Cap `max_tokens=200` sur tous les tests live |

---

## 13. Conclusion

R2 ne réinvente rien. Le code des 6 nano-robots, des 3 providers LLM, du DAG
executor, des workflows mission_a/b/speculative et du Coordinator est **déjà
écrit et testé en isolation**. R2 est l'**opération chirurgicale de wiring
end-to-end** qui prouve que **le pré-mâchage SLM économise réellement des
tokens sur les gros LLMs** quand on enchaîne tout en production réelle.

L'investissement (~33 h) débloque :
1. Un produit **mesurablement plus économique** (–55 % coût $) que toute
   approche LLM-only naïve.
2. Une **différentiation technique** : EFE est la seule couche d'orchestration
   open-source qui combine SLM Phase 1 + multi-LLM Phase 3 avec audit + PII
   + vote + speculation **en un seul workflow déclaratif**.
3. Le **fondement nécessaire** pour Phase 4 (UI), Phase 5 (multi-tenant) et
   Phase 7 (NATS prod) — sans pré-mâchage validé, ces phases tournent à vide.

**Prochaine action concrète** : démarrer Sprint S1 (bootstrap des 4 llama-server
locaux). Estimation : 3 heures. Livrable : `./scripts/start_llama_servers.sh`
fonctionnel + 4 modèles GGUF téléchargés.

---

## Annexe A — Schéma précis des appels HTTP/réseau en Mission A complète

```text
USER QUERY
  │
  ▼
[Coordinator process]
  │
  ├──► HTTP POST localhost:8090/completion  (analyzer)     ~150 ms
  ├──► HTTP POST localhost:8091/completion  (reformulator) ~200 ms  (∥)
  ├──► HTTP POST localhost:8092/completion  (compressor)   ~250 ms  (∥)
  ├──► ONNX local inference (filter bge-m3)                ~80 ms   (∥)
  ├──► local function (router Pareto)                      ~5 ms    (∥)
  │
  ├──► HTTPS POST api.anthropic.com/v1/messages            ~1500 ms
  │     OR
  ├──► HTTPS POST generativelanguage.googleapis.com/v1beta ~600 ms
  │     OR
  ├──► HTTPS POST api.openai.com/v1/chat/completions       ~800 ms
  │
  ├──► HTTP POST localhost:8093/completion  (verifier)     ~300 ms
  │
  ▼
FINAL TEXT + AUDIT CHAIN + STREAM EVENTS
```

Total typique pour Mission A factual_simple : **~2.5 s à 3 s**.

## Annexe B — Glossaire R2

- **SLM** : Small Language Model (< 2B params), Phase 1, local, gratuit
- **LLM** : Large Language Model (gros, distant, payant)
- **Pré-mâchage** : pipeline SLM qui transforme un input brut en input optimisé pour LLM
- **Symphony** : algorithme de vote pondéré multi-LLM (HF 2508.20019)
- **Speculative RAG** : pattern drafter local + verifier distant (HF 2407.08223)
- **DAG** : Directed Acyclic Graph d'exécution parallèle des tâches
- **Phéromone Blackboard** : mémoire partagée bornée 256 KB (inspiration SwarmSys)
- **Audit chain** : chaîne HMAC-SHA256 d'événements pour reproductibilité forensique

---

**Document scellé pour Sprint S1 immédiat. Approbation requise pour démarrer.**
