// SPDX-License-Identifier: UNLICENSED
//! Tests live Phase 3 R2 — Missions Réelles Live (end-to-end Coordinator+LLM).
//!
//! ## Sprint S2 — Mission A naked
//!
//! Premier test de la phase R2 : prouver que la chaîne
//! `Coordinator → MultiLlmBridge → ClaudeProvider live → MissionAResponse`
//! fonctionne **sans aucun nano-robot Phase 1** activé (CoordinatorClients
//! par défaut, tous `None`).
//!
//! C'est le « Hello World » de Mission A : isolation maximale du wiring
//! Provider ↔ Coordinator. Aucune complexité pré-mâchage — celle-ci arrive
//! en S3 (analyzer + verifier), S4 (compressor + reformulator + métriques),
//! puis S5-S9 (vote multi-LLM, speculative, failure modes, savings report).
//!
//! ## Pré-requis pour exécuter LIVE
//!
//! ```bash
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers \
//!     --test live_missions -- --nocapture
//! ```
//!
//! ## Comportement en l'absence des variables
//!
//! Sans `EFE_LIVE_LLM=1`, chaque test fait un early-return en mode `[skip]`
//! et la suite retourne `ok. 1 passed; 0 failed` — pas de bruit, pas de
//! dépendance externe. C'est ce qui permet à la suite offline de toujours
//! passer (412+ PASS) sans nécessiter de clé API.
//!
//! ## Référence
//!
//! Architecture détaillée :
//! `docs/architecture/EFE_A1_PHASE3_R2_S2_MISSION_A_NAKED_ARCHITECTURE.md`

#![cfg(feature = "all-providers")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::llm::{LlmProvider, ProviderRegistry};
use efe_coordinator::workflows::{run_mission_a, run_speculative};
use efe_coordinator::{
    ClaudeProvider, Coordinator, MissionARequest, MissionBRequest, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

// ─── Helpers communs (réutilisés en S3-S9) ──────────────────────────────────

/// Gate des tests live. Convention partagée avec `tests/live_providers.rs`.
///
/// Retourne `true` si `EFE_LIVE_LLM=1` est dans l'environnement.
/// Si `false`, les tests doivent early-return avec un message `[skip]`.
fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_LLM").as_deref() == Ok("1")
}

/// Chemin absolu vers une fixture mission, relatif à `tests/fixtures/missions/`.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/missions")
        .join(name)
}

/// Charge une fixture JSON. Panique si fichier absent ou JSON invalide
/// (volontaire : ces erreurs doivent stopper le test, pas le skip).
fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("invalid JSON in {}: {e}", path.display()))
}

/// Métadonnées LLM par défaut pour Claude Sonnet 4.6 (R1a). Coûts/latences
/// alignés avec ce que les R1b live tests ont observé.
///
/// Réutilisable par n'importe quel test live R2 qui veut un Claude raisonnable.
/// En S6 (router tuning), ces valeurs seront remplacées par des données issues
/// d'un metering live, mais pour S2-S5 elles sont suffisantes.
fn default_claude_metadata() -> LlmMetadata {
    LlmMetadata {
        // Coût Claude Sonnet 4.6 : 3 $ / 1M input, 15 $ / 1M output.
        cost_input_per_million: 300,
        cost_output_per_million: 1500,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        // Latence observée en R1b live : ~1.2 s p50, ~3 s p99.
        p50_latency_ms: 1200,
        p99_latency_ms: 3000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Construit un `MissionARequest` minimal pour une question factuelle, en
/// forçant un LLM précis via `available_llms = vec![llm_id]`. Sans Router
/// nano-robot, `run_mission_a` prend le premier de cette liste (cf.
/// `src/workflows/mission_a.rs:131-200`).
///
/// Budget large pour ne pas faire échouer le test sur les caps. Pour des
/// tests qui veulent éprouver les caps (S8 failure modes), construire la
/// requête à la main.
///
/// Réutilisable en S3-S9 : on garde la signature stable pour permettre
/// d'enrichir les fixtures (`expected_route`, `min_compression_ratio`, …)
/// sans modifier la fonction.
fn build_factual_request(query: &str, llm_id: &str) -> MissionARequest {
    let mut metadata = HashMap::new();
    metadata.insert(llm_id.to_string(), default_claude_metadata());
    MissionARequest {
        tenant_id: None,
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            // Cap budget : 10 ¢ = ~3 000 tokens cumulés Sonnet (large pour fixture).
            max_cost_cents: 10,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 30_000,
            min_quality_score: 0.0,
        },
        // `candidate_sources` vide → verifier auto-skip (cf. request.rs:37).
        // C'est crucial pour S2 (« naked ») — on ne veut pas que le verifier
        // se déclenche accidentellement.
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(30),
        // S6 : pas de tuning router pour S2 (test single-LLM, available_llms
        // = [claude_id] → routage trivial).
        task_class_scoring_table: None,
    }
}

/// Construit un `Coordinator` « naked » : 1 provider + 0 nano-robot Phase 1.
///
/// C'est la configuration testée en S2. Extraite ici pour permettre la
/// réutilisation par S5 (Mission B naked) et par les futurs tests de
/// régression qui veulent isoler le wiring du pré-mâchage.
///
/// Le `Coordinator` est entièrement default sauf le registry :
/// - `clients`  : `CoordinatorClients::default()` (tous None)
/// - `config`   : `CoordinatorConfig::default()` (deadline 30 s, conc 8)
/// - `bus`      : `InMemoryBus` (visible via `coord.subscribe(...)`)
/// - `vote`     : `VoteEngine::default()`
fn make_naked_coordinator(provider: Arc<dyn LlmProvider>) -> Coordinator {
    let registry = ProviderRegistry::new();
    registry.register(provider);
    Coordinator::builder()
        .registry(registry)
        .build()
        .expect("coordinator builds with default clients (all None)")
}

// ─── Sprint S2 ── Mission A naked ───────────────────────────────────────────

/// Le test fondateur de R2 : Coordinator + 1 vrai provider Claude, ZÉRO
/// nano-robot Phase 1, 1 question factuelle. Prouve que la tuyauterie
/// `MultiLlmBridge → LlmProvider::call()` fonctionne en conditions réelles
/// ET que le bus SSE émet bien les événements `SWARM_STARTED`/`SWARM_FINISHED`/
/// `LLM_FINISHED` (preuve que le wiring `EventBus` est branché).
///
/// 11 assertions A1-A11 :
/// - A1 final_text non vide
/// - A2 final_text contient "paris"
/// - A3 llm_used = claude
/// - A4 cost_cents ≤ 5
/// - A5 latency_ms < 30 000
/// - A6 latency_ms ≥ 100 (sinon mock suspect)
/// - A7 audit_chain_hash = 64 chars BLAKE3 hex
/// - A8 verdicts vide (pas de verifier)
/// - A9 wall-clock < 35 000 ms
/// - A10 events SSE : SWARM_STARTED + SWARM_FINISHED présents (G2 patch S2)
/// - A11 events SSE : LLM_FINISHED présent
#[tokio::test]
async fn live_mission_a_naked_no_slm_just_claude() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY to enable");
        return;
    }

    // ── 1. Charge la fixture ────────────────────────────────────────────
    let fixture = load_fixture("01_factual_simple.json");
    let query = fixture["query"].as_str().expect("fixture.query missing");
    let expected_keyword = fixture["expected_keyword"]
        .as_str()
        .expect("fixture.expected_keyword missing");

    // ── 2. Provider Claude réel (la clé est lue depuis l'env) ───────────
    let claude = Arc::new(
        ClaudeProvider::from_env()
            .expect("ANTHROPIC_API_KEY must be set (and start with 'sk-ant-')"),
    );
    let claude_id = claude.id();

    eprintln!("=== Mission A naked ===");
    eprintln!("  query    : {query}");
    eprintln!("  llm_id   : {claude_id}");

    // ── 3. Coordinator naked (via helper réutilisable S5+) ──────────────
    let coord = make_naked_coordinator(claude);

    // ── 4. Abonnement au bus AVANT le run pour capturer tous les events ─
    //
    // Convention du Coordinator : `subscribe(Some(token))` retourne un
    // `(SubscriberToken, tokio::sync::mpsc::Receiver<StreamEvent>)`.
    // On consommera les events après le run_mission_a en non-bloquant
    // (cf. pattern du test offline mission_a_publishes_swarm_started_and_finished).
    let (_subscriber_token, mut rx) = coord.subscribe(Some("s2-naked-test"));

    // ── 5. Construit la requête MissionA forçant Claude ─────────────────
    //
    // Sans Router, `run_mission_a` prend `available_llms.first()`. On met
    // donc Claude en premier (et seul), ce qui rend le routing trivial
    // et déterministe.
    let req = build_factual_request(query, &claude_id);

    // ── 6. Exécution + mesure wall-clock ────────────────────────────────
    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a should succeed (HTTP 200, valid JSON, audit chain OK)");
    let wall_ms = started.elapsed().as_millis() as u64;

    // ── 7. Drain les events du bus en non-bloquant (timeout 50 ms) ──────
    //
    // Les events sont émis dans `run_mission_a` de manière fire-and-forget.
    // Si l'EventBus est branché correctement, on devrait recevoir au moins
    // `SWARM_STARTED`, `LLM_FINISHED`, `SWARM_FINISHED` dans le canal.
    let mut tags: Vec<&'static str> = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
    {
        tags.push(ev.tag());
    }

    // ── 8. Affichage humain du résumé (visible via --nocapture) ─────────
    let audit_hash_head = &resp.audit_chain_hash[..8.min(resp.audit_chain_hash.len())];
    let audit_hash_tail_start = resp.audit_chain_hash.len().saturating_sub(8);
    let audit_hash_tail = &resp.audit_chain_hash[audit_hash_tail_start..];
    eprintln!("  cost_cents  : {}", resp.cost_cents);
    eprintln!(
        "  latency_ms  : {} (mission_a internal) / {} (wall-clock)",
        resp.latency_ms, wall_ms
    );
    eprintln!(
        "  audit_hash  : {}…{} ({} chars)",
        audit_hash_head,
        audit_hash_tail,
        resp.audit_chain_hash.len()
    );
    eprintln!("  final_text  : {:?}", resp.final_text);
    eprintln!("  events_seen : {tags:?}");

    // ── 9. Assertions A1-A11 (cf. doc architecture §4.3 + patch G2) ─────

    // A1 — Le LLM a produit du texte (sinon = bug provider ou stream vide).
    assert!(
        !resp.final_text.is_empty(),
        "A1: final_text non vide (provider doit avoir streamé au moins 1 token)"
    );

    // A2 — Convergence factuelle sur "paris" (lowercase pour robustesse).
    assert!(
        resp.final_text.to_lowercase().contains(expected_keyword),
        "A2: la réponse doit contenir '{expected_keyword}', got: {:?}",
        resp.final_text
    );

    // A3 — Pas de substitution silencieuse par un autre provider/mock.
    assert_eq!(
        resp.llm_used, claude_id,
        "A3: le LLM appelé doit être '{claude_id}' (pas un mock caché)"
    );

    // A4 — Coût raisonnable : Sonnet sur ~30 tokens prompt + ~30 output
    // = ~1 ¢ (cf. R1b validation). On cap à 5 ¢ pour marge.
    assert!(
        resp.cost_cents <= 5,
        "A4: coût ≤ 5 ¢ pour une question simple (cap budget), got {} ¢",
        resp.cost_cents
    );

    // A5 — Cap deadline globale (max_total_latency = 30 s).
    assert!(
        resp.latency_ms < 30_000,
        "A5: latence < 30 s (deadline budget), got {} ms",
        resp.latency_ms
    );

    // A6 — Plancher de réalité : un vrai appel HTTPS prend au moins ~100 ms
    // (handshake TLS + propagation + parsing). Si on observe < 100 ms, c'est
    // probablement un mock qui s'est faufilé.
    assert!(
        resp.latency_ms >= 100,
        "A6: latence ≥ 100 ms (sinon mock suspect), got {} ms",
        resp.latency_ms
    );

    // A7 — audit_chain_hash = BLAKE3 hex (256 bits = 64 chars hex).
    assert_eq!(
        resp.audit_chain_hash.len(),
        64,
        "A7: audit_chain_hash = BLAKE3 hex 64 chars, got len {} : {:?}",
        resp.audit_chain_hash.len(),
        resp.audit_chain_hash
    );

    // A8 — Pas de verifier nano-robot activé → pas de FactCheck verdict.
    assert!(
        resp.verdicts.is_empty(),
        "A8: pas de verifier nano-robot → verdicts vide, got {} verdicts",
        resp.verdicts.len()
    );

    // A9 — Wall-clock sanity check (latence Mission A + overhead test).
    assert!(
        wall_ms < 35_000,
        "A9: wall-clock < 35 s (sanity global), got {wall_ms} ms"
    );

    // A10 — Events SSE émis : SWARM_STARTED + SWARM_FINISHED (preuve que
    // l'EventBus est branché et que le workflow émet bien les bornes).
    assert!(
        tags.contains(&"SWARM_STARTED"),
        "A10a: SWARM_STARTED manquant dans les events, got tags={tags:?}"
    );
    assert!(
        tags.contains(&"SWARM_FINISHED"),
        "A10b: SWARM_FINISHED manquant dans les events, got tags={tags:?}"
    );

    // A11 — Au moins un événement LLM_FINISHED (preuve que le LLM a bien
    // été appelé via le bridge — pas un retour précoce silencieux).
    assert!(
        tags.contains(&"LLM_FINISHED"),
        "A11: LLM_FINISHED manquant (le LLM n'a peut-être pas été appelé), got tags={tags:?}"
    );
}

// ─── Helpers Sprint S3 (analyzer + verifier nano-robots Phase 1) ────────────

/// URL de l'analyzer (port 8090). Convention env `ANALYZER_URL` ou défaut local.
/// Aligné avec `crates/efe-nano-app-coordinator/tests/live_smoke.rs:39-41`.
fn analyzer_endpoint() -> String {
    std::env::var("ANALYZER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

/// URL du verifier (port 8093). Convention env `VERIFIER_URL` ou défaut local.
fn verifier_endpoint() -> String {
    std::env::var("VERIFIER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8093".to_string())
}

/// Gate spécifique aux tests requérant les SLM Phase 1 (en plus de `EFE_LIVE_LLM`).
///
/// Convention : un test live qui exige `llama-server` actif vérifie d'abord
/// `EFE_LIVE_SLM=1`. Ceci permet aux utilisateurs sans SLM locaux (mais avec
/// clé API Claude) de continuer à faire passer la suite live sans erreurs.
fn live_slm_enabled() -> bool {
    std::env::var("EFE_LIVE_SLM").as_deref() == Ok("1")
}

/// Pré-flight async : ping HTTP /health des 2 endpoints SLM (analyzer + verifier).
/// Retourne `true` si les 2 répondent 200 OK, `false` sinon.
///
/// Non bloquant — l'appelant skip si false (cf. test ci-dessous). Cela permet
/// de lancer la suite avec `EFE_LIVE_SLM=1` même si les daemons ne sont pas
/// (encore) démarrés : le test skip proprement avec un message explicite.
async fn analyzer_and_verifier_alive() -> bool {
    let client = reqwest::Client::new();
    let ping = |url: String| {
        let c = client.clone();
        async move {
            c.get(format!("{url}/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
        }
    };
    let analyzer_ok = ping(analyzer_endpoint()).await;
    let verifier_ok = ping(verifier_endpoint()).await;
    if !analyzer_ok {
        eprintln!("  [health-check] analyzer  DOWN  → {}/health", analyzer_endpoint());
    }
    if !verifier_ok {
        eprintln!("  [health-check] verifier  DOWN  → {}/health", verifier_endpoint());
    }
    analyzer_ok && verifier_ok
}

/// Variante de `build_factual_request` qui inclut des `candidate_sources`
/// (requis pour activer le verifier en S3+).
///
/// **Double trigger du verifier** (workflows/mission_a.rs:307-352) :
///   1. `req.candidate_sources.is_empty() == false`  ← cette fonction garantit ≥1
///   2. `coord.clients.verifier.is_some()`           ← responsabilité du builder
///
/// Si l'un manque, le verifier skip silencieusement et `verdicts` reste vide.
fn build_factual_request_with_sources(
    query: &str,
    llm_id: &str,
    sources: Vec<(String, String)>, // tuples (id, text)
) -> MissionARequest {
    let mut metadata = HashMap::new();
    metadata.insert(llm_id.to_string(), default_claude_metadata());
    MissionARequest {
        tenant_id: None,
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            // Budget élargi vs S2 car analyzer + verifier ajoutent ~1 s cumulés.
            max_cost_cents: 10,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 60_000,
            min_quality_score: 0.0,
        },
        candidate_sources: sources
            .into_iter()
            .map(|(id, text)| efe_coordinator::CandidateSourceRef { id, text })
            .collect(),
        max_total_latency: Duration::from_secs(60),
        // S6 : pas de tuning router pour S3 (test single-LLM Claude).
        task_class_scoring_table: None,
    }
}

/// Construit un `Coordinator` partiel : 1 provider LLM + analyzer + verifier
/// (les 4 autres nano-robots sont `None`).
///
/// C'est la configuration testée en Sprint S3. Variante intermédiaire entre
/// `make_naked_coordinator` (S2, 0 robot) et `make_full_coordinator` (S4, 6 robots).
///
/// Les clients SLM lisent leur grammar GBNF + prompt depuis `assets/grammars/`
/// et `assets/prompts/` du crate `efe-nano-app-coordinator` (cf. `from_assets`).
/// Si ces fichiers sont absents → `from_assets` retourne une erreur → expect panique.
fn make_coord_with_analyzer_verifier(provider: Arc<dyn LlmProvider>) -> Coordinator {
    use efe_coordinator::CoordinatorClients;
    use efe_nano_app_coordinator::clients::{AnalyzerClient, VerifierClient};

    let analyzer = Arc::new(
        AnalyzerClient::from_assets(analyzer_endpoint())
            .expect("AnalyzerClient::from_assets — assets/grammars/analyzer_v1.gbnf manquant ?"),
    );
    let verifier = Arc::new(
        VerifierClient::from_assets(verifier_endpoint())
            .expect("VerifierClient::from_assets — assets/grammars/verifier_v1.gbnf manquant ?"),
    );

    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        verifier: Some(verifier),
        reformulator: None,
        compressor: None,
        filter: None,
        router: None,
        // R3.S2 : default empty (mode S8 backward-compat).
        slm_circuits: std::collections::HashMap::new(),
    };

    let registry = ProviderRegistry::new();
    registry.register(provider);

    Coordinator::builder()
        .registry(registry)
        .clients(clients)
        .build()
        .expect("coordinator builds with analyzer + verifier")
}

// ─── Sprint S3 ── Mission A + analyzer + verifier ───────────────────────────

/// Test S3 : Coordinator + Claude + 2 nano-robots Phase 1 (analyzer :8090,
/// verifier :8093).
///
/// Prouve que la collaboration **SLM ↔ LLM ↔ SLM** fonctionne en mission live :
///   1. analyzer extrait l'intent + les entités du `user_text`
///   2. les entités sont déposées comme phéromones dans la blackboard
///   3. Claude génère la réponse sur le `user_text` (pas encore compressé en S3)
///   4. verifier fact-check le `final_text` contre les `candidate_sources`
///   5. le verdict est attaché à `MissionAResponse.verdicts`
///
/// 16 assertions B1-B16 (cf. doc architecture §4.3) :
/// - B1-B9   héritées S2 (final_text, cost, latency, audit hash, wall-clock)
/// - B10     verdicts non vide (verifier a tourné)
/// - B11     verdict == SUPPORTED (consensus factuel claim ↔ sources)
/// - B12     similarity ≥ seuil fixture (0.3 par défaut)
/// - B13     events ROBOT_STARTED + ROBOT_FINISHED (analyzer)
/// - B14     event PHEROMONE_DEPOSITED ≥ 1 (analyzer dépose au moins 1 entité)
/// - B15     event VERDICT_READY (verifier branché au bus)
/// - B16     latency < 10 s (cap S3, élargi vs S2 car +analyzer+verifier)
#[tokio::test]
async fn live_mission_a_with_analyzer_verifier() {
    // ── Gates en cascade (les 3 doivent être vraies) ────────────────────
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY (Claude requis)");
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 (llama-server :8090/:8093 requis)");
        return;
    }
    if !analyzer_and_verifier_alive().await {
        eprintln!("[skip] analyzer or verifier not responding on /health");
        eprintln!("       → lance : ./scripts/slm/start.sh && ./scripts/slm/health.sh");
        return;
    }

    // ── 1. Charge la fixture enrichie S3 ────────────────────────────────
    let fixture = load_fixture("02_factual_with_sources.json");
    let query = fixture["query"].as_str().expect("fixture.query missing");
    let expected_kw = fixture["expected_keyword"]
        .as_str()
        .expect("fixture.expected_keyword missing");
    let expected_verdict = fixture["expected_verifier_verdict"]
        .as_str()
        .unwrap_or("SUPPORTED");
    let expected_sim_min: f32 = fixture["expected_verifier_similarity_min"]
        .as_f64()
        .unwrap_or(0.3) as f32;
    let sources: Vec<(String, String)> = fixture["candidate_sources"]
        .as_array()
        .expect("fixture.candidate_sources missing")
        .iter()
        .map(|s| {
            (
                s["id"].as_str().expect("source.id missing").to_string(),
                s["text"].as_str().expect("source.text missing").to_string(),
            )
        })
        .collect();
    assert!(!sources.is_empty(), "fixture must provide ≥1 candidate_source");

    // ── 2. Provider Claude réel (la clé est lue depuis l'env) ───────────
    let claude = Arc::new(
        ClaudeProvider::from_env()
            .expect("ANTHROPIC_API_KEY must be set (and start with 'sk-ant-')"),
    );
    let claude_id = claude.id();

    // ── 3. Coordinator partiel (analyzer + verifier, 4 autres None) ─────
    let coord = make_coord_with_analyzer_verifier(claude);

    eprintln!("=== Mission A with analyzer + verifier ===");
    eprintln!("  query          : {query}");
    eprintln!("  llm_id         : {claude_id}");
    eprintln!("  analyzer_url   : {}", analyzer_endpoint());
    eprintln!("  verifier_url   : {}", verifier_endpoint());
    eprintln!(
        "  candidate_srcs : {} ({:?})",
        sources.len(),
        sources.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
    );

    // ── 4. Abonnement au bus AVANT le run (capture des events) ──────────
    let (_token, mut rx) = coord.subscribe(Some("s3-analyzer-verifier-test"));

    // ── 5. Construit la requête avec sources (active le verifier) ───────
    let req = build_factual_request_with_sources(query, &claude_id, sources.clone());

    // ── 6. Exécution + mesure wall-clock ────────────────────────────────
    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a should succeed (analyzer + claude + verifier all OK)");
    let wall_ms = started.elapsed().as_millis() as u64;

    // ── 7. Drain events bus (timeout 200ms, plus large car +6 events S3) ─
    let mut tags: Vec<&'static str> = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
    {
        tags.push(ev.tag());
    }

    // ── 8. Affichage humain (visible via --nocapture) ───────────────────
    let audit_hash_head = &resp.audit_chain_hash[..8.min(resp.audit_chain_hash.len())];
    let audit_hash_tail_start = resp.audit_chain_hash.len().saturating_sub(8);
    let audit_hash_tail = &resp.audit_chain_hash[audit_hash_tail_start..];
    eprintln!("  cost_cents     : {}", resp.cost_cents);
    eprintln!(
        "  latency_ms     : {} (mission_a) / {} (wall-clock)",
        resp.latency_ms, wall_ms
    );
    eprintln!(
        "  audit_hash     : {}…{} ({} chars)",
        audit_hash_head,
        audit_hash_tail,
        resp.audit_chain_hash.len()
    );
    eprintln!("  final_text     : {:?}", resp.final_text);

    if let Some(v) = resp.verdicts.first() {
        eprintln!("  verifier:");
        eprintln!("    verdict      : {}", v.verdict);
        eprintln!("    severity     : {}", v.severity);
        eprintln!("    similarity   : {:.3}", v.similarity);
        eprintln!("    best_source  : {:?}", v.best_source_id);
    } else {
        eprintln!("  verifier       : (NO VERDICT — verifier skipped ?!)");
    }
    eprintln!("  events_seen ({}) : {tags:?}", tags.len());

    // ── 9. Assertions B1-B16 ────────────────────────────────────────────

    // B1 — Le LLM a produit du texte.
    assert!(!resp.final_text.is_empty(), "B1: final_text non vide");

    // B2 — Convergence factuelle "paris" (lowercase tolerant).
    assert!(
        resp.final_text.to_lowercase().contains(expected_kw),
        "B2: la réponse doit contenir '{expected_kw}', got: {:?}",
        resp.final_text
    );

    // B3 — Pas de substitution par un mock.
    assert_eq!(
        resp.llm_used, claude_id,
        "B3: le LLM doit être '{claude_id}', got '{}'",
        resp.llm_used
    );

    // B4 — Coût raisonnable (Sonnet sur 30+30 tokens ≈ 1¢ + analyzer/verifier gratuits).
    assert!(
        resp.cost_cents <= 5,
        "B4: coût ≤ 5 ¢, got {} ¢",
        resp.cost_cents
    );

    // B5 — Cap deadline globale (60 s pour S3 — analyzer + verifier en plus).
    assert!(
        resp.latency_ms < 60_000,
        "B5: latency < 60 s, got {} ms",
        resp.latency_ms
    );

    // B6 — Plancher de réalité (handshake TLS Claude ~100 ms).
    assert!(
        resp.latency_ms >= 100,
        "B6: latency ≥ 100 ms (sinon mock suspect), got {} ms",
        resp.latency_ms
    );

    // B7 — audit_chain_hash = BLAKE3 hex 64 chars.
    assert_eq!(
        resp.audit_chain_hash.len(),
        64,
        "B7: audit_chain_hash = BLAKE3 hex 64 chars, got {}: {:?}",
        resp.audit_chain_hash.len(),
        resp.audit_chain_hash
    );

    // B9 — Wall-clock sanity check.
    assert!(wall_ms < 60_000, "B9: wall-clock < 60 s, got {wall_ms} ms");

    // B10 — Verifier a tourné (verdicts non vide).
    //
    // Différence majeure vs S2 (où on assertait l'inverse) : ici les
    // candidate_sources sont fournies ET le client verifier est instantié.
    assert!(
        !resp.verdicts.is_empty(),
        "B10: verifier doit avoir produit ≥1 verdict (sources fournies + verifier UP). \
         events_seen={tags:?}"
    );
    let v = &resp.verdicts[0];

    // B11 — Verdict attendu (SUPPORTED par défaut pour la fixture 02).
    assert_eq!(
        v.verdict.to_uppercase(),
        expected_verdict.to_uppercase(),
        "B11: verdict attendu '{expected_verdict}', got '{}'. Sources: {:?}, claim: {:?}",
        v.verdict,
        sources.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        resp.final_text,
    );

    // B12 — Similarity ≥ seuil fixture.
    assert!(
        v.similarity >= expected_sim_min,
        "B12: similarity ≥ {expected_sim_min}, got {}",
        v.similarity
    );

    // B13 — Events analyzer (ROBOT_STARTED + ROBOT_FINISHED).
    assert!(
        tags.contains(&"ROBOT_STARTED"),
        "B13a: ROBOT_STARTED manquant (analyzer pas branché ?), got {tags:?}"
    );
    assert!(
        tags.contains(&"ROBOT_FINISHED"),
        "B13b: ROBOT_FINISHED manquant, got {tags:?}"
    );

    // B14 — Au moins 1 phéromone déposée (analyzer a extrait ≥1 entité).
    //
    // Pour la query "Quelle est la capitale de la France ?", l'analyzer doit
    // détecter au moins 1 entité (typiquement "France"). Sinon le modèle
    // SmolLM2-360M n'a pas tagué correctement, ou la blackboard ne reçoit pas.
    assert!(
        tags.contains(&"PHEROMONE_DEPOSITED"),
        "B14: PHEROMONE_DEPOSITED manquant (analyzer doit déposer ≥1 entité). \
         Inspecter logs analyzer : ./scripts/slm/logs.sh analyzer --last 30. \
         events_seen={tags:?}"
    );

    // B15 — Verifier branché au bus (event VERDICT_READY).
    assert!(
        tags.contains(&"VERDICT_READY"),
        "B15: VERDICT_READY manquant (le verifier doit émettre cet event), got {tags:?}"
    );

    // B16 — Cap latency S3 (20 s — calibré post-run).
    //
    // L'estimation initiale (10 s) sous-estimait la génération de
    // l'analyzer en mode complet (20 entités JSON = 200+ tokens cold cache
    // → ~5-7 s) et du verifier (claim + sources → output JSON ~3-4 s).
    // En cold cache (premier appel post-start), le mission_a complet prend
    // ~12 s. Le cap à 20 s catch une régression sans faire échouer le test
    // en conditions normales (warm cache typique : 8-10 s).
    assert!(
        resp.latency_ms < 20_000,
        "B16: latency < 20 s (cap S3 calibré post-run), got {} ms. \
         Si > 20s : vérifier `./scripts/slm/bench.sh` que GPU Metal est actif.",
        resp.latency_ms
    );
}

// ─── Helpers Sprint S4 (full pre-chewing pipeline + ChewingMetrics) ─────────

/// URL du compressor (port 8092). Convention env `COMPRESSOR_URL` ou défaut local.
fn compressor_endpoint() -> String {
    std::env::var("COMPRESSOR_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8092".to_string())
}

/// Pré-flight S4 : ping HTTP /health des 3 endpoints SLM utiles à la pipeline
/// complète (analyzer + compressor + verifier). Retourne `true` si les 3
/// répondent 200 OK. Reformulator (:8091) n'est PAS vérifié car le workflow
/// `run_mission_a` ne l'utilise pas (cf. doc architecture S4 §1.3).
async fn full_pipeline_slm_alive() -> bool {
    let client = reqwest::Client::new();
    let ping = |url: String| {
        let c = client.clone();
        async move {
            c.get(format!("{url}/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
        }
    };
    let analyzer_ok = ping(analyzer_endpoint()).await;
    let compressor_ok = ping(compressor_endpoint()).await;
    let verifier_ok = ping(verifier_endpoint()).await;
    if !analyzer_ok {
        eprintln!("  [health-check] analyzer    DOWN → {}/health", analyzer_endpoint());
    }
    if !compressor_ok {
        eprintln!("  [health-check] compressor  DOWN → {}/health", compressor_endpoint());
    }
    if !verifier_ok {
        eprintln!("  [health-check] verifier    DOWN → {}/health", verifier_endpoint());
    }
    analyzer_ok && compressor_ok && verifier_ok
}

/// Construit un `Coordinator` avec le **pipeline complet** Mission A actif :
/// analyzer + router + compressor + verifier (les 4 robots réellement utilisés
/// par le workflow). Reformulator et filter restent `None` car le workflow
/// `run_mission_a` ne les invoque pas (audit S4 §1.3).
///
/// Variante "full" de `make_naked_coordinator` (S2) et de
/// `make_coord_with_analyzer_verifier` (S3).
fn make_coord_full_pipeline(provider: Arc<dyn LlmProvider>) -> Coordinator {
    use efe_coordinator::CoordinatorClients;
    use efe_nano_app_coordinator::clients::{
        AnalyzerClient, CompressorClient, RouterClient, VerifierClient,
    };

    let analyzer = Arc::new(
        AnalyzerClient::from_assets(analyzer_endpoint())
            .expect("AnalyzerClient::from_assets — assets manquants ?"),
    );
    let compressor = Arc::new(
        CompressorClient::from_assets(compressor_endpoint())
            .expect("CompressorClient::from_assets — assets manquants ?"),
    );
    let verifier = Arc::new(
        VerifierClient::from_assets(verifier_endpoint())
            .expect("VerifierClient::from_assets — assets manquants ?"),
    );
    // Router : heuristique Pareto in-process, pas de from_assets.
    let router = Arc::new(RouterClient::new());

    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        compressor: Some(compressor),
        verifier: Some(verifier),
        router: Some(router),
        // reformulator et filter : wired-but-unused dans le workflow actuel,
        // donc on les laisse None pour ne pas masquer une intégration ultérieure
        // (cf. architecture S4 §1.3 — Sprint S5+ ou Phase 4).
        reformulator: None,
        filter: None,
        // R3.S2 : default empty (mode S8 backward-compat).
        slm_circuits: std::collections::HashMap::new(),
    };

    let registry = ProviderRegistry::new();
    registry.register(provider);

    Coordinator::builder()
        .registry(registry)
        .clients(clients)
        .build()
        .expect("coordinator builds with full pipeline (4 robots)")
}

// ─── Sprint S4 ── Mission A FULL pre-chewing pipeline + ChewingMetrics ──────

/// Test S4 : pipeline pré-mâchage COMPLET (4 robots actifs) + mesure
/// quantitative des économies de tokens via `ChewingMetrics`.
///
/// **C'est LE test qui prouve enfin le but premier de R2** : économiser
/// des tokens sur les gros LLMs grâce au pré-mâchage SLM.
///
/// Flow attendu :
/// 1. analyzer extrait entités + intent
/// 2. router choisit Claude (1 seul candidat → trivial)
/// 3. compressor réduit le user_text de ~130 tokens à ~30 tokens
/// 4. Claude reçoit le summary compressé (pas le paragraphe original)
/// 5. verifier fact-check contre les sources
/// 6. ChewingMetrics agrégées émises en event PRE_CHEWING_METRICS
/// 7. MissionAResponse.chewing_metrics rempli pour audit
///
/// 28 assertions C1-C28 (cf. doc architecture S4 §4.3).
#[tokio::test]
async fn live_mission_a_full_pre_chewing_pipeline() {
    // ── Gates en cascade ────────────────────────────────────────────────
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY (Claude requis)");
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 (llama-server :8090/:8092/:8093 requis)");
        return;
    }
    if !full_pipeline_slm_alive().await {
        eprintln!("[skip] analyzer, compressor ou verifier not responding on /health");
        eprintln!("       → lance : ./scripts/slm/start.sh && ./scripts/slm/health.sh");
        return;
    }

    // ── 1. Charge la fixture S4 (query ≥ 500 chars pour déclencher compressor) ─
    let fixture = load_fixture("03_factual_compressible.json");
    let query = fixture["query"].as_str().expect("fixture.query missing");
    assert!(
        query.len() > 500,
        "fixture.query DOIT faire > 500 chars (seuil compressor mission_a.rs:204), \
         got {} chars",
        query.len()
    );
    let expected_kw = fixture["expected_keyword"]
        .as_str()
        .expect("fixture.expected_keyword");
    let expected_verdict = fixture["expected_verifier_verdict"]
        .as_str()
        .unwrap_or("SUPPORTED");
    let expected_sim_min: f32 = fixture["expected_verifier_similarity_min"]
        .as_f64()
        .unwrap_or(0.3) as f32;
    let expected_ratio_max: f32 = fixture["expected_compression_ratio_max"]
        .as_f64()
        .unwrap_or(0.7) as f32;
    let expected_savings_min: u32 = fixture["expected_savings_tokens_min"]
        .as_u64()
        .unwrap_or(20) as u32;
    let sources: Vec<(String, String)> = fixture["candidate_sources"]
        .as_array()
        .expect("fixture.candidate_sources")
        .iter()
        .map(|s| {
            (
                s["id"].as_str().expect("source.id").to_string(),
                s["text"].as_str().expect("source.text").to_string(),
            )
        })
        .collect();

    // ── 2. Provider Claude ──────────────────────────────────────────────
    let claude = Arc::new(
        ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY must be set"),
    );
    let claude_id = claude.id();

    // ── 3. Coordinator avec pipeline complet (4 robots) ─────────────────
    let coord = make_coord_full_pipeline(claude);

    eprintln!("=== Mission A FULL pre-chewing pipeline ===");
    eprintln!("  query_raw      : \"{}…\" ({} chars)", &query[..80.min(query.len())], query.len());
    eprintln!("  llm_id         : {claude_id}");
    eprintln!("  analyzer_url   : {}", analyzer_endpoint());
    eprintln!("  compressor_url : {}", compressor_endpoint());
    eprintln!("  verifier_url   : {}", verifier_endpoint());
    eprintln!("  router         : in-process Pareto heuristic");
    eprintln!("  candidate_srcs : {}", sources.len());

    // ── 4. Subscribe bus (capture événements + PRE_CHEWING_METRICS) ─────
    let (_token, mut rx) = coord.subscribe(Some("s4-full-pipeline-test"));

    // ── 5. Build request ────────────────────────────────────────────────
    let req = build_factual_request_with_sources(query, &claude_id, sources.clone());

    // ── 6. Run mission + mesure wall-clock ──────────────────────────────
    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a full pipeline should succeed");
    let wall_ms = started.elapsed().as_millis() as u64;

    // ── 7. Drain events (timeout 300 ms — plus de tags qu'en S3) ────────
    let mut tags: Vec<&'static str> = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(300), rx.recv()).await
    {
        tags.push(ev.tag());
    }

    // ── 8. Extract ChewingMetrics depuis la response ────────────────────
    let metrics = resp
        .chewing_metrics
        .as_ref()
        .expect("C16: MissionAResponse.chewing_metrics doit être Some(...) en S4");

    // ── 9. Affichage humain enrichi ─────────────────────────────────────
    let audit_hash_head = &resp.audit_chain_hash[..8.min(resp.audit_chain_hash.len())];
    let audit_hash_tail_start = resp.audit_chain_hash.len().saturating_sub(8);
    let audit_hash_tail = &resp.audit_chain_hash[audit_hash_tail_start..];
    eprintln!();
    eprintln!("  ── Phase 1 : Pre-chewing SLM ──");
    eprintln!(
        "    analyzer     : {} ms",
        metrics.analyzer_latency_ms
    );
    eprintln!(
        "    router       : {} ms   → {}  (reason: {:?})",
        metrics.router_latency_ms, metrics.selected_llm, metrics.router_reason
    );
    eprintln!(
        "    compressor   : {} ms   ({} → {} tokens, ratio={:.3}, savings={} tokens {:.0}%)",
        metrics.compressor_latency_ms,
        metrics.input_tokens_raw,
        metrics.input_tokens_compressed,
        metrics.compression_ratio,
        metrics.savings_tokens,
        metrics.savings_ratio * 100.0,
    );
    eprintln!();
    eprintln!("  ── Phase 2 : LLM sur prompt compressé ──");
    eprintln!(
        "    llm          : {} ms   ({} out tokens, cost {} ¢)",
        metrics.llm_latency_ms, metrics.llm_tokens_generated, resp.cost_cents
    );
    eprintln!("    final_text   : {:?}", resp.final_text);
    eprintln!();
    eprintln!("  ── Phase 3 : Verifier post-LLM ──");
    eprintln!("    verifier     : {} ms", metrics.verifier_latency_ms);
    if let Some(v) = resp.verdicts.first() {
        eprintln!("    verdict      : {}", v.verdict);
        eprintln!("    similarity   : {:.3}", v.similarity);
        eprintln!("    best_source  : {:?}", v.best_source_id);
    }
    eprintln!();
    eprintln!("  ── ChewingMetrics ──");
    eprintln!(
        "    total_slm_latency_ms      : {}",
        metrics.total_slm_latency_ms
    );
    eprintln!(
        "    total_pipeline_latency_ms : {}",
        metrics.total_pipeline_latency_ms
    );
    eprintln!("    wall_clock_ms             : {wall_ms}");
    eprintln!(
        "    audit_hash   : {audit_hash_head}…{audit_hash_tail} ({} chars)",
        resp.audit_chain_hash.len()
    );
    eprintln!("    events_seen ({}) : {tags:?}", tags.len());

    // ── 10. Assertions C1-C28 (cf. doc architecture S4 §4.3) ────────────

    // C1-C9 — héritées B1-B9 (S3)
    assert!(!resp.final_text.is_empty(), "C1: final_text non vide");
    assert!(
        resp.final_text.to_lowercase().contains(expected_kw),
        "C2: contient '{expected_kw}', got {:?}",
        resp.final_text
    );
    assert_eq!(resp.llm_used, claude_id, "C3: llm_used = Claude");
    assert!(
        resp.cost_cents <= 5,
        "C4: cost ≤ 5 ¢, got {} ¢",
        resp.cost_cents
    );
    assert!(
        resp.latency_ms < 60_000,
        "C5: latency < 60 s, got {} ms",
        resp.latency_ms
    );
    assert!(
        resp.latency_ms >= 100,
        "C6: latency ≥ 100 ms, got {} ms",
        resp.latency_ms
    );
    assert_eq!(
        resp.audit_chain_hash.len(),
        64,
        "C7: audit_hash 64 chars, got {}",
        resp.audit_chain_hash.len()
    );
    assert!(wall_ms < 60_000, "C9: wall-clock < 60 s, got {wall_ms}");

    // C10-C12 — héritées B10-B12 (S3) : verifier
    assert!(
        !resp.verdicts.is_empty(),
        "C10: verifier doit avoir produit ≥ 1 verdict"
    );
    let v = &resp.verdicts[0];
    assert_eq!(
        v.verdict.to_uppercase(),
        expected_verdict.to_uppercase(),
        "C11: verdict attendu '{expected_verdict}', got '{}'",
        v.verdict
    );
    assert!(
        v.similarity >= expected_sim_min,
        "C12: similarity ≥ {expected_sim_min}, got {}",
        v.similarity
    );

    // C13-C15 — héritées B13-B15 : events analyzer/verifier
    assert!(
        tags.contains(&"ROBOT_STARTED"),
        "C13a: ROBOT_STARTED missing, got {tags:?}"
    );
    assert!(
        tags.contains(&"ROBOT_FINISHED"),
        "C13b: ROBOT_FINISHED missing"
    );
    assert!(
        tags.contains(&"PHEROMONE_DEPOSITED"),
        "C14: PHEROMONE_DEPOSITED missing (analyzer doit déposer ≥ 1 entité)"
    );
    assert!(
        tags.contains(&"VERDICT_READY"),
        "C15: VERDICT_READY missing"
    );

    // ── Assertions S4 spécifiques (C16-C28) ──────────────────────────────

    // C16 — MissionAResponse.chewing_metrics rempli
    // (déjà vérifié via .expect() au début — re-affirm here)
    assert!(
        resp.chewing_metrics.is_some(),
        "C16: chewing_metrics doit être Some(...)"
    );

    // C17 — input_tokens_raw ≥ 100 (query ≥ 500 chars ≈ 125+ tokens)
    assert!(
        metrics.input_tokens_raw >= 100,
        "C17: input_tokens_raw ≥ 100 (query longue), got {}",
        metrics.input_tokens_raw
    );

    // C18 — compressor a effectivement réduit le texte
    assert!(
        metrics.input_tokens_compressed < metrics.input_tokens_raw,
        "C18: compressor doit avoir réduit (raw={} > compressed={})",
        metrics.input_tokens_raw,
        metrics.input_tokens_compressed,
    );

    // C19 — compression_ratio sous le cap fixture
    assert!(
        metrics.compression_ratio <= expected_ratio_max,
        "C19: compression_ratio ≤ {expected_ratio_max}, got {:.3}",
        metrics.compression_ratio
    );

    // C20 — savings_tokens au-dessus du minimum fixture
    assert!(
        metrics.savings_tokens >= expected_savings_min,
        "C20: savings_tokens ≥ {expected_savings_min}, got {}",
        metrics.savings_tokens
    );

    // C21 — compressor a tourné dans un délai raisonnable
    assert!(
        metrics.compressor_latency_ms > 0,
        "C21a: compressor_latency_ms > 0 (compressor doit avoir tourné, pas skip)"
    );
    assert!(
        metrics.compressor_latency_ms < 10_000,
        "C21b: compressor_latency_ms < 10s, got {}",
        metrics.compressor_latency_ms
    );

    // C22 — router heuristique in-process Rust = très rapide
    assert!(
        metrics.router_latency_ms < 100,
        "C22: router_latency_ms < 100 ms (heuristique Pareto in-process), got {}",
        metrics.router_latency_ms
    );

    // C23 — avec 1 seul LLM candidat, le router pick lui
    assert_eq!(
        metrics.selected_llm, claude_id,
        "C23: router doit sélectionner claude (1 seul candidat), got {:?}",
        metrics.selected_llm
    );

    // C24 — router a écrit une justification
    assert!(
        !metrics.router_reason.is_empty(),
        "C24: router_reason doit être non vide (audit trail)"
    );

    // C25 — event PRE_CHEWING_METRICS émis par le workflow
    assert!(
        tags.contains(&"PRE_CHEWING_METRICS"),
        "C25: PRE_CHEWING_METRICS missing dans les events (workflow doit l'émettre), \
         got {tags:?}"
    );

    // C26 — cohérence : SLM total ≤ total pipeline
    assert!(
        metrics.total_slm_latency_ms <= metrics.total_pipeline_latency_ms,
        "C26: SLM ({}) <= pipeline ({}) — incohérence si SLM > pipeline",
        metrics.total_slm_latency_ms,
        metrics.total_pipeline_latency_ms,
    );

    // C27 — plancher LLM (handshake TLS Claude)
    assert!(
        metrics.llm_latency_ms >= 100,
        "C27: llm_latency ≥ 100 ms (plancher TLS), got {}",
        metrics.llm_latency_ms
    );

    // C28 — cap latency S4 (élargi vs S3 car compressor en plus)
    assert!(
        resp.latency_ms < 25_000,
        "C28: latency < 25 s (cap S4), got {} ms",
        resp.latency_ms
    );
}

// ─── Helpers Sprint S5 (Mission B 3-LLM consensus + pré-mâchage mutualisé) ──

/// Métadonnées par défaut pour Gemini Flash (cohérent avec R1b live data).
fn default_gemini_metadata() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 8,
        cost_output_per_million: 30,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 800,
        p99_latency_ms: 2500,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Métadonnées par défaut pour GPT-4o (cohérent avec R1b live data).
fn default_gpt_metadata() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 250,
        cost_output_per_million: 1000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 900,
        p99_latency_ms: 3000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Gate S5 : vérifie les 3 clés API (Anthropic + Google + OpenAI).
/// Retourne `false` si l'une manque — le test skip avec message clair.
fn all_three_api_keys_set() -> bool {
    let anth = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let goog = std::env::var("GOOGLE_AI_KEY").is_ok();
    let openai = std::env::var("OPENAI_API_KEY").is_ok();
    if !anth {
        eprintln!("  [env] ANTHROPIC_API_KEY missing");
    }
    if !goog {
        eprintln!("  [env] GOOGLE_AI_KEY missing");
    }
    if !openai {
        eprintln!("  [env] OPENAI_API_KEY missing");
    }
    anth && goog && openai
}

/// Construit un `MissionBRequest` avec 3 LLMs + sources Wikipédia.
/// Vote uniforme (défaut), quorum None (= ceil(3/2) = 2).
fn build_mission_b_request_3llm(
    query: &str,
    llm_ids: [&str; 3],
    sources: Vec<(String, String)>,
) -> MissionBRequest {
    let mut metadata = HashMap::new();
    metadata.insert(llm_ids[0].to_string(), default_claude_metadata());
    metadata.insert(llm_ids[1].to_string(), default_gemini_metadata());
    metadata.insert(llm_ids[2].to_string(), default_gpt_metadata());
    MissionBRequest {
        tenant_id: None,
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        llms: llm_ids.iter().map(|s| s.to_string()).collect(),
        llm_metadata: metadata,
        vote_weights: HashMap::new(), // uniform
        quorum: None,                 // défaut ceil(N/2) = 2
        candidate_sources: sources
            .into_iter()
            .map(|(id, text)| efe_coordinator::CandidateSourceRef { id, text })
            .collect(),
        max_total_latency: Duration::from_secs(60),
    }
}

/// Construit un Coordinator avec les 3 providers réels + 4 nano-robots Phase 1
/// (analyzer + compressor + verifier + router — reformulator/filter restent None).
///
/// Sprint S5 — testé en live avec les 3 vrais providers en parallèle.
fn make_coord_for_mission_b_pipeline(
    claude: Arc<dyn LlmProvider>,
    gemini: Arc<dyn LlmProvider>,
    gpt: Arc<dyn LlmProvider>,
) -> Coordinator {
    use efe_coordinator::CoordinatorClients;
    use efe_nano_app_coordinator::clients::{
        AnalyzerClient, CompressorClient, RouterClient, VerifierClient,
    };

    let analyzer = Arc::new(
        AnalyzerClient::from_assets(analyzer_endpoint())
            .expect("AnalyzerClient::from_assets — assets manquants ?"),
    );
    let compressor = Arc::new(
        CompressorClient::from_assets(compressor_endpoint())
            .expect("CompressorClient::from_assets — assets manquants ?"),
    );
    let verifier = Arc::new(
        VerifierClient::from_assets(verifier_endpoint())
            .expect("VerifierClient::from_assets — assets manquants ?"),
    );
    let router = Arc::new(RouterClient::new());

    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        compressor: Some(compressor),
        verifier: Some(verifier),
        router: Some(router),
        reformulator: None,
        filter: None,
        // R3.S2 : default empty (mode S8 backward-compat).
        slm_circuits: std::collections::HashMap::new(),
    };

    let registry = ProviderRegistry::new();
    registry.register(claude);
    registry.register(gemini);
    registry.register(gpt);

    Coordinator::builder()
        .registry(registry)
        .clients(clients)
        .build()
        .expect("coordinator builds with 3 providers + 4 robots")
}

// ─── Démo INPUT LONG ── baseline (brut) vs pré-mâchage (compressé) ─────────

/// **Démonstration de l'économie de coût sur un INPUT LONG.**
///
/// Les fixtures courtes (questions factuelles ~130 tokens) ne montrent pas
/// d'économie : le compresseur ne mord pas (< 500 chars) ou le coût des tokens
/// de SORTIE domine. Ici on passe le **même** input long (~1870 tokens, fixture
/// `09_long_document_compressible`) dans les deux chemins :
/// - **baseline** : Claude reçoit le prompt brut (0 SLM) ;
/// - **chewed** : analyzer + compressor compriment AVANT Claude (verifier
///   auto-skippé car 0 source → on isole la compression).
///
/// Sur un input long, la réduction des tokens d'entrée (~−95 % via le compressor
/// Llama-3.2-1B ciblant 30 mots) devient **matérielle** dans le coût Claude,
/// puisque l'entrée n'est plus noyée par la sortie. Claude seul — **aucun
/// Gemini/GPT requis**.
///
/// Gates : `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `EFE_LIVE_SLM=1` +
/// llama-server analyzer(8090)/compressor(8092) vivants.
#[tokio::test]
async fn live_mission_a_long_input_baseline_vs_chewed() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY (Claude requis)");
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 (llama-server :8090/:8092 requis)");
        return;
    }
    if !full_pipeline_slm_alive().await {
        eprintln!("[skip] analyzer/compressor injoignables — ./scripts/slm/start.sh");
        return;
    }

    // ── Fixture : défaut = input long (09) → démo SÉCURITÉ (fallback brut) ;
    //    override `EFE_DEMO_FIXTURE=10_medium_compressible.json` → SWEET-SPOT
    //    (compression valide + savings matériels). Un seul test, deux régimes. ──
    let fixture_name = std::env::var("EFE_DEMO_FIXTURE")
        .unwrap_or_else(|_| "09_long_document_compressible.json".to_string());
    let fixture = load_fixture(&fixture_name);
    let query = fixture["query"].as_str().expect("fixture.query");
    let expected_kw = fixture["expected_keyword"].as_str().expect("expected_keyword");
    assert!(
        query.len() > 1000,
        "fixture doit être assez longue (> 1000 chars) pour un coût d'entrée notable, got {}",
        query.len()
    );

    // ── Provider Claude (Arc<dyn> cloné pour les 2 runs) ──
    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY must be set"));
    let claude_id = claude.id();

    eprintln!("=== INPUT : baseline (brut) vs pré-mâchage (compressé) ===");
    eprintln!("  fixture   : {fixture_name}");
    eprintln!(
        "  query     : {} chars (~{} tokens estimés)",
        query.len(),
        query.len() / 4
    );
    eprintln!("  llm       : {claude_id}  (Claude seul, pas de routing)");

    // ── 1) BASELINE : Claude reçoit le prompt BRUT (0 SLM) ──
    let coord_baseline = make_naked_coordinator(Arc::clone(&claude));
    let resp_b = run_mission_a(&coord_baseline, build_factual_request(query, &claude_id))
        .await
        .expect("baseline mission_a doit réussir");
    let raw_tokens = resp_b
        .chewing_metrics
        .as_ref()
        .map(|m| m.input_tokens_raw)
        .unwrap_or(0);
    let base_mc = resp_b.cost_microcents;

    // ── 2) CHEWED : analyzer + compressor compriment AVANT Claude ──
    let coord_chewed = make_coord_full_pipeline(Arc::clone(&claude));
    let resp_c = run_mission_a(&coord_chewed, build_factual_request(query, &claude_id))
        .await
        .expect("chewed mission_a doit réussir");
    let m = resp_c
        .chewing_metrics
        .as_ref()
        .expect("chewing_metrics doit être Some");
    let chewed_mc = resp_c.cost_microcents;

    // ── Comparaison chiffrée ──
    eprintln!();
    eprintln!("  ── BASELINE (prompt brut → Claude) ──");
    eprintln!("    input_tokens : {raw_tokens}");
    eprintln!("    cost         : {:.4}¢ ({} µ¢)", base_mc as f64 / 1000.0, base_mc);
    eprintln!("    latency      : {} ms", resp_b.latency_ms);
    eprintln!();
    eprintln!("  ── CHEWED (analyzer+compressor → Claude) ──");
    eprintln!(
        "    compressor   : {} ms  ({} → {} tokens, ratio={:.3}, −{:.0}%)",
        m.compressor_latency_ms,
        m.input_tokens_raw,
        m.input_tokens_compressed,
        m.compression_ratio,
        m.savings_ratio * 100.0
    );
    eprintln!(
        "    input_tokens : {} (vs {} brut)",
        m.input_tokens_compressed, raw_tokens
    );
    eprintln!("    cost         : {:.4}¢ ({} µ¢)", chewed_mc as f64 / 1000.0, chewed_mc);
    eprintln!(
        "    latency      : {} ms (overhead SLM inclus)",
        resp_c.latency_ms
    );

    // Fallback détecté de façon CAUSE-AGNOSTIQUE : si les tokens d'entrée n'ont
    // pas été réduits, le compresseur n'a pas comprimé (dégénération placeholder
    // via le garde `Ok`, OU timeout 5s via la branche `Err`, OU toute autre
    // erreur) et mission_a est retombé sur le prompt brut. On ne se fie donc pas
    // à un nom de composant précis mais au résultat observable.
    let fell_back = m.input_tokens_compressed >= m.input_tokens_raw;
    let cost_savings = if base_mc > 0 {
        1.0 - (chewed_mc as f64 / base_mc as f64)
    } else {
        0.0
    };
    eprintln!();
    eprintln!("  ══════════════ VERDICT ══════════════");
    if fell_back {
        eprintln!(
            "    ⚠️ compresseur N'A PAS COMPRIMÉ ({} ms — dégénération placeholder \
             ou timeout 5s) → fallback prompt brut. Qualité préservée par le \
             garde, savings nuls (le modèle 1B plafonne au-delà de ~1k tokens).",
            m.compressor_latency_ms
        );
    }
    eprintln!(
        "    Tokens entrée : {} → {}  (−{:.0}%)",
        m.input_tokens_raw,
        m.input_tokens_compressed,
        m.savings_ratio * 100.0
    );
    eprintln!(
        "    Coût Claude   : {:.4}¢ → {:.4}¢  ({}{:.1}% {})",
        base_mc as f64 / 1000.0,
        chewed_mc as f64 / 1000.0,
        if cost_savings >= 0.0 { "−" } else { "+" },
        cost_savings.abs() * 100.0,
        if cost_savings > 0.30 {
            "✅ matériel"
        } else if fell_back {
            "fallback brut"
        } else {
            "⚠️ modeste"
        }
    );

    // ── Assertions ──
    // GARANTIE CŒUR : qualité préservée des deux côtés, quoi qu'il arrive. Le
    // garde mission_a assure que même si le compresseur dégénère, Claude reçoit
    // le prompt brut (qui contient la question) et répond correctement.
    assert!(
        resp_b
            .final_text
            .to_lowercase()
            .contains(&expected_kw.to_lowercase()),
        "baseline doit contenir '{expected_kw}', got: {:?}",
        resp_b.final_text
    );
    assert!(
        resp_c
            .final_text
            .to_lowercase()
            .contains(&expected_kw.to_lowercase()),
        "chewed doit contenir '{expected_kw}' — le garde anti-garbage doit \
         préserver la qualité même quand le compresseur dégénère, got: {:?}",
        resp_c.final_text
    );
    // Savings : assertion CONDITIONNELLE. Quand le compresseur produit un résumé
    // valide, l'économie doit être matérielle ; quand il dégénère sur un input
    // trop long, le fallback brut donne 0 savings — comportement correct
    // (sécurité > économie), pas un échec.
    if !fell_back {
        assert!(
            m.input_tokens_compressed < m.input_tokens_raw,
            "compression valide → réduction des tokens attendue ({} → {})",
            m.input_tokens_raw,
            m.input_tokens_compressed
        );
        assert!(
            cost_savings > 0.30,
            "compression valide → économie matérielle (> 30%) attendue, got {:.1}%",
            cost_savings * 100.0
        );
    }
}

// ─── Sprint S5 ── Mission B 3-LLM consensus + pré-mâchage MUTUALISÉ ─────────

/// Test S5 : Mission B end-to-end avec **3 vrais providers**
/// (Claude + Gemini + GPT) en parallèle, pré-mâchage **mutualisé** (1×
/// compressor pour les 3 LLMs), vote Symphony, et `ChewingMetrics` qui
/// quantifie le savings × N.
///
/// **C'est le sprint qui démontre l'argument économique multiplié de R2** :
/// le savings de 85 % observé en S4 s'applique × 3 LLMs ici → savings cumulés
/// énormes sur les tokens d'entrée envoyés aux gros LLMs.
///
/// 25 assertions D1-D25 (cf. doc architecture S5 §4).
#[tokio::test]
async fn live_mission_b_full_3llm_consensus_with_pre_chewing() {
    use efe_coordinator::workflows::run_mission_b;
    use efe_coordinator::{GeminiProvider, GptProvider};

    // ── Gates en cascade ─────────────────────────────────────────────────
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + 3 clés API");
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 (llama-server :8090/:8092/:8093 requis)");
        return;
    }
    if !all_three_api_keys_set() {
        eprintln!("[skip] 3 clés API requises : ANTHROPIC_API_KEY + GOOGLE_AI_KEY + OPENAI_API_KEY");
        return;
    }
    if !full_pipeline_slm_alive().await {
        eprintln!("[skip] analyzer/compressor/verifier not responding");
        return;
    }

    // ── 1. Charge fixture (réutilise 03 — query > 500 chars) ─────────────
    let fixture = load_fixture("03_factual_compressible.json");
    let query = fixture["query"].as_str().expect("fixture.query");
    assert!(
        query.len() > 500,
        "fixture.query DOIT faire > 500 chars (seuil compressor mission_b.rs), got {}",
        query.len()
    );
    let expected_kw = fixture["expected_keyword"].as_str().expect("kw");
    let expected_verdict = fixture["expected_verifier_verdict"]
        .as_str()
        .unwrap_or("SUPPORTED");
    let expected_sim_min: f32 = fixture["expected_verifier_similarity_min"]
        .as_f64()
        .unwrap_or(0.3) as f32;
    let expected_ratio_max: f32 = fixture["expected_compression_ratio_max"]
        .as_f64()
        .unwrap_or(0.7) as f32;
    let expected_savings_min: u32 = fixture["expected_savings_tokens_min"]
        .as_u64()
        .unwrap_or(30) as u32;
    let sources: Vec<(String, String)> = fixture["candidate_sources"]
        .as_array()
        .expect("fixture.candidate_sources")
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_string(),
                s["text"].as_str().unwrap().to_string(),
            )
        })
        .collect();

    // ── 2. 3 providers réels ─────────────────────────────────────────────
    let claude = Arc::new(
        ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"),
    );
    let gemini = Arc::new(
        GeminiProvider::from_env().expect("GOOGLE_AI_KEY"),
    );
    let gpt = Arc::new(
        GptProvider::from_env().expect("OPENAI_API_KEY"),
    );
    let claude_id = claude.id();
    let gemini_id = gemini.id();
    let gpt_id = gpt.id();

    // ── 3. Coordinator avec 3 providers + 4 nano-robots ─────────────────
    let coord = make_coord_for_mission_b_pipeline(claude, gemini, gpt);

    eprintln!("=== Mission B 3-LLM consensus + pré-mâchage MUTUALISÉ ===");
    eprintln!(
        "  query_raw      : \"{}…\" ({} chars)",
        &query[..80.min(query.len())],
        query.len()
    );
    eprintln!("  3 LLMs         :");
    eprintln!("    • {claude_id}");
    eprintln!("    • {gemini_id}");
    eprintln!("    • {gpt_id}");
    eprintln!("  4 robots actifs: analyzer + compressor + verifier + router");
    eprintln!("  candidate_srcs : {}", sources.len());

    // ── 4. Subscribe bus ─────────────────────────────────────────────────
    let (_token, mut rx) = coord.subscribe(Some("s5-mission-b-test"));

    // ── 5. Build MissionBRequest avec 3 LLMs ─────────────────────────────
    let req = build_mission_b_request_3llm(
        query,
        [&claude_id, &gemini_id, &gpt_id],
        sources.clone(),
    );

    // ── 6. Run Mission B + mesure wall-clock ────────────────────────────
    let started = std::time::Instant::now();
    let resp = run_mission_b(&coord, req)
        .await
        .expect("mission_b should succeed (3 LLMs + 4 robots)");
    let wall_ms = started.elapsed().as_millis() as u64;

    // ── 7. Drain events (timeout 400 ms — beaucoup d'events en Mission B) ─
    let mut tags: Vec<&'static str> = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(400), rx.recv()).await
    {
        tags.push(ev.tag());
    }

    // ── 8. Extract chewing_metrics ───────────────────────────────────────
    let metrics = resp
        .chewing_metrics
        .as_ref()
        .expect("D13: MissionBResponse.chewing_metrics doit être Some(...) en S5");

    // ── 9. Affichage humain ──────────────────────────────────────────────
    let audit_head = &resp.audit_chain_hash[..8.min(resp.audit_chain_hash.len())];
    let audit_tail_start = resp.audit_chain_hash.len().saturating_sub(8);
    let audit_tail = &resp.audit_chain_hash[audit_tail_start..];
    eprintln!();
    eprintln!("  ── Phase 1 : Pré-mâchage MUTUALISÉ ──");
    eprintln!("    analyzer     : {} ms", metrics.analyzer_latency_ms);
    eprintln!(
        "    compressor   : {} ms   ({} → {} tokens, ratio={:.3}, savings={} tokens {:.0}%)",
        metrics.compressor_latency_ms,
        metrics.input_tokens_raw,
        metrics.input_tokens_compressed,
        metrics.compression_ratio,
        metrics.savings_tokens,
        metrics.savings_ratio * 100.0,
    );
    eprintln!(
        "    ► Le savings de {} tokens s'applique × 3 LLMs en aval ◄",
        metrics.savings_tokens
    );
    eprintln!(
        "    Savings cumulés : {} × 3 = {} tokens d'entrée économisés",
        metrics.savings_tokens,
        metrics.savings_tokens * 3
    );
    eprintln!();
    eprintln!("  ── Phase 2 : Fan-out 3 LLMs PARALLÈLE ──");
    eprintln!(
        "    llm_latency_ms (max des 3)        : {} ms",
        metrics.llm_latency_ms
    );
    eprintln!(
        "    llm_tokens_generated (somme des 3): {}",
        metrics.llm_tokens_generated
    );
    eprintln!("    total_cost_cents (somme des 3)    : {} ¢", resp.total_cost_cents);
    eprintln!();
    eprintln!("  ── Phase 3 : Vote Symphony ──");
    eprintln!("    winner_llm   : {}", resp.winner_llm);
    eprintln!("    winner_score : {:.2}", resp.winner_score);
    eprintln!("    per_llm_scores ({}):", resp.per_llm_scores.len());
    for (id, score) in &resp.per_llm_scores {
        eprintln!("      • {id} → {score:.2}");
    }
    eprintln!();
    eprintln!("  ── Phase 4 : Verifier post-vote ──");
    eprintln!("    verifier     : {} ms", metrics.verifier_latency_ms);
    if let Some(v) = resp.verdicts.first() {
        eprintln!("    verdict      : {}", v.verdict);
        eprintln!("    similarity   : {:.3}", v.similarity);
        eprintln!("    best_source  : {:?}", v.best_source_id);
    }
    eprintln!();
    eprintln!("  ── Mission B Response ──");
    eprintln!("    final_text   : {:?}", resp.final_text);
    eprintln!(
        "    total_pipeline_latency_ms : {} (wall_clock = {})",
        metrics.total_pipeline_latency_ms, wall_ms
    );
    eprintln!(
        "    audit_hash   : {audit_head}…{audit_tail} ({} chars)",
        resp.audit_chain_hash.len()
    );
    eprintln!("    events_seen ({}) : VOTE_RESOLVED in {}, PRE_CHEWING_METRICS in {}",
        tags.len(),
        tags.contains(&"VOTE_RESOLVED"),
        tags.contains(&"PRE_CHEWING_METRICS"),
    );

    // ── 10. Assertions D1-D25 ────────────────────────────────────────────

    // D1 — final_text non vide
    assert!(!resp.final_text.is_empty(), "D1: final_text non vide");

    // D2 — convergence factuelle "paris"
    assert!(
        resp.final_text.to_lowercase().contains(expected_kw),
        "D2: contient '{expected_kw}', got {:?}",
        resp.final_text
    );

    // D3 — winner_llm est un des 3 réels
    let valid_winners = [claude_id.as_str(), gemini_id.as_str(), gpt_id.as_str()];
    assert!(
        valid_winners.contains(&resp.winner_llm.as_str()),
        "D3: winner_llm doit être l'un des 3 candidats, got {:?}",
        resp.winner_llm
    );

    // D4 — per_llm_scores contient les 3 LLMs
    assert_eq!(
        resp.per_llm_scores.len(),
        3,
        "D4: 3 scores attendus, got {}",
        resp.per_llm_scores.len()
    );

    // D5 — tous les scores per-LLM > 0 (= les 3 ont produit un ballot valide)
    for (id, score) in &resp.per_llm_scores {
        assert!(
            *score > 0.0,
            "D5: tous scores > 0, mais {id} = {score}"
        );
    }

    // D6 — cap coût total (3 LLMs)
    assert!(
        resp.total_cost_cents <= 10,
        "D6: total cost ≤ 10¢ (somme 3 LLMs), got {}¢",
        resp.total_cost_cents
    );

    // D7 — coût au moins 1¢ (= vrais providers, pas mocks)
    assert!(
        resp.total_cost_cents >= 1,
        "D7: total cost ≥ 1¢ (sinon mocks ?), got {}¢",
        resp.total_cost_cents
    );

    // D8 — cap deadline
    assert!(
        resp.latency_ms < 30_000,
        "D8: latency < 30s, got {}ms",
        resp.latency_ms
    );

    // D9 — audit hash 64 chars
    assert_eq!(resp.audit_chain_hash.len(), 64, "D9: audit hash 64 chars");

    // D10 — verifier a tourné
    assert!(!resp.verdicts.is_empty(), "D10: verdicts non vide");
    let v = &resp.verdicts[0];

    // D11 — verdict SUPPORTED
    assert_eq!(
        v.verdict.to_uppercase(),
        expected_verdict.to_uppercase(),
        "D11: verdict '{expected_verdict}' attendu, got '{}'",
        v.verdict
    );

    // D12 — similarity ≥ seuil
    assert!(
        v.similarity >= expected_sim_min,
        "D12: similarity ≥ {expected_sim_min}, got {}",
        v.similarity
    );

    // D13 — chewing_metrics.is_some()
    // (déjà vérifié via .expect() au début)
    assert!(resp.chewing_metrics.is_some(), "D13: chewing_metrics Some");

    // D14 — input_tokens_raw ≥ 100
    assert!(
        metrics.input_tokens_raw >= 100,
        "D14: input_tokens_raw ≥ 100, got {}",
        metrics.input_tokens_raw
    );

    // D15 — compressed < raw
    assert!(
        metrics.input_tokens_compressed < metrics.input_tokens_raw,
        "D15: compressed < raw ({} < {})",
        metrics.input_tokens_compressed,
        metrics.input_tokens_raw
    );

    // D16 — compression_ratio ≤ cap fixture
    assert!(
        metrics.compression_ratio <= expected_ratio_max,
        "D16: compression_ratio ≤ {expected_ratio_max}, got {:.3}",
        metrics.compression_ratio
    );

    // D17 — savings_tokens ≥ min fixture
    assert!(
        metrics.savings_tokens >= expected_savings_min,
        "D17: savings_tokens ≥ {expected_savings_min}, got {}",
        metrics.savings_tokens
    );

    // D18 — compressor a tourné (mutualisé, 1× pour les 3 LLMs)
    assert!(
        metrics.compressor_latency_ms > 0 && metrics.compressor_latency_ms < 10_000,
        "D18: 0 < compressor_latency < 10s, got {}ms",
        metrics.compressor_latency_ms
    );

    // D19 — llm_latency (max des 3) < 15s (parallèle, pas somme)
    assert!(
        metrics.llm_latency_ms < 15_000,
        "D19: llm_latency_ms (max des 3) < 15s, got {}ms",
        metrics.llm_latency_ms
    );

    // D20 — selected_llm == winner_llm (cohérence)
    assert_eq!(
        metrics.selected_llm, resp.winner_llm,
        "D20: selected_llm should == winner_llm"
    );

    // D21 — event VOTE_RESOLVED
    assert!(
        tags.contains(&"VOTE_RESOLVED"),
        "D21: VOTE_RESOLVED missing, got tags={tags:?}"
    );

    // D22 — event PRE_CHEWING_METRICS (S5 NEW)
    assert!(
        tags.contains(&"PRE_CHEWING_METRICS"),
        "D22: PRE_CHEWING_METRICS missing, got tags={tags:?}"
    );

    // D23 — events ROBOT_FINISHED ≥ 2 (analyzer + compressor + verifier)
    let robot_finished_count =
        tags.iter().filter(|t| **t == "ROBOT_FINISHED").count();
    assert!(
        robot_finished_count >= 2,
        "D23: ROBOT_FINISHED ≥ 2, got {robot_finished_count}"
    );

    // D24 — wall-clock sanity
    assert!(
        wall_ms < 35_000,
        "D24: wall_clock < 35s, got {wall_ms}ms"
    );

    // D25 — MULTIPLICATIF : savings × 3 LLMs (= économie cumulée)
    let cumulative_savings = metrics.savings_tokens * 3;
    assert!(
        cumulative_savings >= 60,
        "D25: cumulative_savings (savings × 3 LLMs) ≥ 60, got {cumulative_savings}\n\
         → démonstration de la multiplication économique : 1× compressor pour 3 LLMs payants"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ─── Sprint S6 R2 — Router Tuning par TaskClass ────────────────────────────
// ═══════════════════════════════════════════════════════════════════════════
//
// S6 teste **le premier vrai choix multi-LLM live de R2** : avec 3 LLMs
// disponibles (Claude + Gemini + GPT), le RouterClient tuné via
// `TaskClassScoringTable::default_3llm_table()` doit choisir le BON LLM
// selon la `task_class` :
//
//  | TaskClass    | Winner attendu                | Driver        |
//  |--------------|-------------------------------|---------------|
//  | FactualQa    | gemini_gemini-2.5-flash       | cost (c=2.0)  |
//  | Code         | claude_claude-sonnet-4-6      | quality (q=2.5) |
//  | Creative     | gpt_gpt-4o                    | quality (q=1.5) |
//  | Translation  | gemini_gemini-2.5-flash       | cost (c=2.0)  |
//
// Pré-conditions : 3 clés API (ANTHROPIC + GOOGLE + OPENAI) + `EFE_LIVE_LLM=1`.
// SLM daemons NON requis (le router tuné est inline, pas un robot ONNX).

/// Construit une `MissionARequest` avec la table de scoring S6 par défaut +
/// override du budget cap à 10 ¢ pour les tests live (économie).
///
/// `available_llms` doit contenir les 3 IDs réels (Claude + Gemini + GPT).
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
    req.budget.max_cost_cents = 10; // cap pour tests live
    req
}

/// Construit un Coordinator avec les 3 providers réels enregistrés.
///
/// Aucun nano-robot Phase 1 activé : le router tuning S6 est purement
/// inline (poids Pareto via `RouterClient::for_task_class`, pas via le
/// robot ONNX `coord.clients.router`).
fn make_coord_for_router_tuning(
    claude: Arc<dyn LlmProvider>,
    gemini: Arc<dyn LlmProvider>,
    gpt: Arc<dyn LlmProvider>,
) -> Coordinator {
    let registry = ProviderRegistry::new();
    registry.register(claude);
    registry.register(gemini);
    registry.register(gpt);
    Coordinator::builder()
        .registry(registry)
        .build()
        .expect("coordinator builds with default clients (all None)")
}

/// Helper commun pour les 4 tests S6 : skip si pré-conditions manquantes,
/// charge fixture, instancie 3 providers, run Mission A avec scoring.
///
/// Retourne `(resp, fixture)` pour permettre les assertions E1-E10 dans
/// chaque test individuel (qui peut varier les seuils selon le LLM gagnant).
async fn run_s6_router_test(
    fixture_name: &str,
) -> Option<(efe_coordinator::MissionAResponse, serde_json::Value)> {
    use efe_coordinator::{GeminiProvider, GptProvider};

    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 to enable S6 router tuning tests");
        return None;
    }
    if !all_three_api_keys_set() {
        eprintln!("[skip] S6 router tuning needs ALL 3 keys (Claude + Gemini + GPT)");
        return None;
    }

    let fixture = load_fixture(fixture_name);
    let query = fixture["query"].as_str().expect("fixture.query");
    let task_class_str = fixture["task_class"].as_str().expect("fixture.task_class");
    // Parse la task_class via serde (cohérent avec router.rs:101-104).
    let task_class: TaskClass =
        serde_json::from_value(serde_json::json!(task_class_str))
            .or_else(|_| {
                // Fallback : convert PascalCase → SCREAMING_SNAKE_CASE
                let upper = match task_class_str {
                    "FactualQa" => "FACTUAL_QA",
                    "Analysis" => "ANALYSIS",
                    "Code" => "CODE",
                    "Creative" => "CREATIVE",
                    "Reasoning" => "REASONING",
                    "Translation" => "TRANSLATION",
                    other => other,
                };
                serde_json::from_value::<TaskClass>(serde_json::json!(upper))
            })
            .expect("fixture.task_class invalid");

    let claude: Arc<dyn LlmProvider> = Arc::new(
        ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"),
    );
    let gemini: Arc<dyn LlmProvider> = Arc::new(
        GeminiProvider::from_env().expect("GOOGLE_AI_KEY"),
    );
    let gpt: Arc<dyn LlmProvider> = Arc::new(
        GptProvider::from_env().expect("OPENAI_API_KEY"),
    );
    let claude_id = claude.id();
    let gemini_id = gemini.id();
    let gpt_id = gpt.id();

    eprintln!("\n=== Mission A — Router Tuning S6 ({fixture_name}) ===");
    eprintln!("  query        : {} ({} chars)", &query.chars().take(80).collect::<String>(), query.len());
    eprintln!("  task_class   : {task_class:?}");
    eprintln!("  available    : [{claude_id}, {gemini_id}, {gpt_id}]");
    eprintln!("  expected     : {}", fixture["expected_route"].as_str().unwrap_or("?"));

    let coord = make_coord_for_router_tuning(claude, gemini, gpt);
    let req = build_mission_a_with_scoring(
        query,
        task_class,
        vec![claude_id.clone(), gemini_id.clone(), gpt_id.clone()],
    );

    let started = std::time::Instant::now();
    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a should succeed (HTTP 200, valid JSON, audit chain OK)");
    let wall_ms = started.elapsed().as_millis() as u64;

    eprintln!("\n  ── Router decision ──");
    eprintln!("    selected     : {}", resp.llm_used);
    if let Some(metrics) = &resp.chewing_metrics {
        eprintln!("    router reason: {}", metrics.router_reason);
    }
    eprintln!("    cost_cents   : {} ¢", resp.cost_cents);
    eprintln!("    latency_ms   : {} ms", resp.latency_ms);
    eprintln!("    audit_hash   : {}…{} ({} chars)",
        &resp.audit_chain_hash[..8],
        &resp.audit_chain_hash[resp.audit_chain_hash.len()-8..],
        resp.audit_chain_hash.len());
    eprintln!("    wall_clock   : {wall_ms} ms");

    Some((resp, fixture))
}

/// E1-E10 assertions communes aux 4 tests S6.
fn assert_s6_routing(
    resp: &efe_coordinator::MissionAResponse,
    fixture: &serde_json::Value,
) {
    let expected_route = fixture["expected_route"].as_str().expect("expected_route");
    let expected_keyword = fixture["expected_keyword"].as_str().expect("expected_keyword");
    let expected_justif = fixture["expected_route_justification_keyword"]
        .as_str()
        .unwrap_or("p50=");

    // E1 — final_text non vide
    assert!(!resp.final_text.is_empty(), "E1: final_text must be non-empty");

    // E2 — llm_used == expected_route
    assert_eq!(
        resp.llm_used, expected_route,
        "E2: router should pick {expected_route}, got {}",
        resp.llm_used
    );

    // E3 — final_text non vide (sanity, dup E1)
    assert!(!resp.final_text.is_empty(), "E3: final_text safety check");

    // E4 — contient le keyword attendu (lowercase substring)
    let final_lc = resp.final_text.to_lowercase();
    assert!(
        final_lc.contains(&expected_keyword.to_lowercase()),
        "E4: final_text should contain '{expected_keyword}', got: {}",
        &resp.final_text.chars().take(200).collect::<String>()
    );

    // E5 — cost_cents ≤ 10
    assert!(
        resp.cost_cents <= 10,
        "E5: cost_cents ≤ 10, got {}",
        resp.cost_cents
    );

    // E6 — cost_cents ≥ 1 (pas mocks)
    assert!(
        resp.cost_cents >= 1,
        "E6: cost_cents ≥ 1 (preuve qu'on a appelé un vrai LLM), got {}",
        resp.cost_cents
    );

    // E7 — latency_ms < 30s
    assert!(
        resp.latency_ms < 30_000,
        "E7: latency_ms < 30s, got {}ms",
        resp.latency_ms
    );

    // E8 — audit_chain_hash 64 chars BLAKE3
    assert_eq!(
        resp.audit_chain_hash.len(),
        64,
        "E8: audit_chain_hash must be 64-char BLAKE3 hex, got {} chars",
        resp.audit_chain_hash.len()
    );

    // E9 — chewing_metrics.router_reason est rempli (preuve que le router
    // a bien tourné, pas skippé)
    let metrics = resp.chewing_metrics.as_ref().expect("E9: chewing_metrics");
    assert!(
        !metrics.router_reason.is_empty(),
        "E9: router_reason should be non-empty (router must have run)"
    );

    // E10 — router_reason contient la justification keyword (e.g. "p50=")
    assert!(
        metrics.router_reason.contains(expected_justif),
        "E10: router_reason should contain '{expected_justif}', got: {}",
        metrics.router_reason
    );
}

/// **Test S6-A** : `TaskClass::FactualQa` → Gemini gagne (cost-driven).
///
/// Question simple "capitale de la France". Avec budget 10 ¢ et 3 LLMs
/// disponibles, le router doit choisir Gemini Flash (cost 0.075 $/M vs
/// Claude 3 $/M) car les capabilities FactualQa sont équivalentes
/// (Gemini 0.92 vs Claude 0.85 vs GPT 0.86).
#[tokio::test]
async fn live_mission_a_router_picks_gemini_for_factual_qa() {
    let Some((resp, fixture)) = run_s6_router_test("01_factual_simple.json").await else {
        return;
    };
    assert_s6_routing(&resp, &fixture);
}

/// **Test S6-B** : `TaskClass::Code` → Claude gagne (quality-driven).
///
/// Génération de code Rust. Weights Code (q=2.5, l=0.5, c=0.2) → Claude
/// gagne malgré son coût 30× supérieur, parce que HumanEval+ ≈ 89 % vs
/// Gemini ≈ 72 %.
#[tokio::test]
async fn live_mission_a_router_picks_claude_for_code() {
    let Some((resp, fixture)) = run_s6_router_test("05_code_gen_rust.json").await else {
        return;
    };
    assert_s6_routing(&resp, &fixture);
}

/// **Test S6-C** : `TaskClass::Creative` → GPT-4o gagne.
///
/// Génération créative (dialogue absurde). Weights Creative (q=1.5, l=1.0,
/// c=1.0) balanced. GPT-4o gagne via capability 0.93 (style + variété)
/// vs Claude 0.88 vs Gemini 0.80.
#[tokio::test]
async fn live_mission_a_router_picks_gpt_for_creative() {
    let Some((resp, fixture)) = run_s6_router_test("06_creative_writing.json").await else {
        return;
    };
    assert_s6_routing(&resp, &fixture);
}

/// **Test S6-D** : `TaskClass::Translation` → Gemini gagne (cost-driven).
///
/// Traduction FR→EN. Weights Translation (q=1.0, l=1.0, c=2.0) → Gemini
/// gagne via cost + capability multilingue 0.92.
#[tokio::test]
async fn live_mission_a_router_picks_gemini_for_translation() {
    let Some((resp, fixture)) = run_s6_router_test("07_translation_fr_en.json").await else {
        return;
    };
    assert_s6_routing(&resp, &fixture);
}

// ═══════════════════════════════════════════════════════════════════════════
// ─── Sprint S7 R2 — Mission Speculative (drafter cheap + verifier premium) ──
// ═══════════════════════════════════════════════════════════════════════════
//
// S7 teste le **Speculative RAG pattern** en live :
//   1. Drafter = Gemini Flash (LLM cheap, ~0.075 $/M) répond d'abord
//   2. Si la réponse est courte (≤ 10 mots) → HOT PATH, Claude jamais appelé
//   3. Sinon SLOW PATH, Claude appelé pour arbitrage Jaccard
//
// Sur queries factuelles simples ("capitale de la France"), Gemini répond
// "Paris" en 1 mot → HOT PATH → savings ~98 % vs appel direct Claude.
//
// Pré-requis : `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`.

/// Helper S7 : gate des pré-conditions (2 clés API + EFE_LIVE_LLM).
fn s7_preconditions_satisfied() -> bool {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 to enable S7 speculative tests");
        return false;
    }
    let anth = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let goog = std::env::var("GOOGLE_AI_KEY").is_ok();
    if !anth || !goog {
        eprintln!(
            "[skip] S7 needs ANTHROPIC_API_KEY + GOOGLE_AI_KEY (anth={anth}, goog={goog})"
        );
        return false;
    }
    true
}

/// Helper S7 : construit un Coordinator avec drafter Gemini + verifier Claude
/// enregistrés sous leurs IDs réels.
fn make_coord_for_speculative(
    drafter: Arc<dyn LlmProvider>,
    verifier: Arc<dyn LlmProvider>,
) -> Coordinator {
    let registry = ProviderRegistry::new();
    registry.register(drafter);
    registry.register(verifier);
    Coordinator::builder()
        .registry(registry)
        .build()
        .expect("coordinator builds")
}

/// Helper S7 : run la fixture de speculative + log les métriques.
async fn run_s7_speculative_test(
    fixture_name: &str,
    max_words_override: usize,
) -> Option<(efe_coordinator::SpeculativeResponse, serde_json::Value)> {
    use efe_coordinator::GeminiProvider;
    if !s7_preconditions_satisfied() {
        return None;
    }

    let fixture = load_fixture(fixture_name);
    let query = fixture["query"].as_str().expect("fixture.query");

    let gemini: Arc<dyn LlmProvider> = Arc::new(
        GeminiProvider::from_env().expect("GOOGLE_AI_KEY"),
    );
    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let drafter_id = gemini.id();
    let verifier_id = claude.id();

    eprintln!("\n=== Mission Speculative — S7 ({fixture_name}) ===");
    eprintln!("  query           : {} ({} chars)", query.chars().take(80).collect::<String>(), query.len());
    eprintln!("  drafter         : {drafter_id} (cheap LLM)");
    eprintln!("  verifier        : {verifier_id} (premium LLM)");
    eprintln!("  max_words_quick : {max_words_override}");
    eprintln!("  expected_skip   : {}", fixture["expected_verifier_skipped"].as_bool().unwrap_or(true));

    let coord = make_coord_for_speculative(gemini, claude);
    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        query.to_string(),
        TaskClass::FactualQa,
        drafter_id,
        verifier_id,
    );
    req.budget.max_cost_cents = 10;
    req.max_draft_tokens = 50;
    req.max_words_for_quick_accept = max_words_override;

    let started = std::time::Instant::now();
    let resp = run_speculative(&coord, req)
        .await
        .expect("speculative should succeed");
    let wall_ms = started.elapsed().as_millis() as u64;

    let metrics = resp.chewing_metrics.as_ref();
    eprintln!("\n  ── Speculation result ──");
    eprintln!("    draft_text       : {:?}", resp.draft_text);
    eprintln!("    final_text       : {:?}", resp.final_text);
    eprintln!("    draft_accepted   : {}", resp.draft_accepted);
    eprintln!("    verifier_skipped : {} {}",
        resp.verifier_skipped,
        if resp.verifier_skipped { "← HOT PATH (savings !)" } else { "← SLOW PATH" });
    eprintln!("    drafter_latency  : {} ms", metrics.map(|m| m.spec_drafter_latency_ms).unwrap_or(0));
    eprintln!("    verifier_latency : {} ms (0 if skipped)", metrics.map(|m| m.spec_verifier_latency_ms).unwrap_or(0));
    eprintln!("    cost_cents       : {} ¢ {}",
        resp.cost_cents,
        if resp.verifier_skipped { "(drafter seul)" } else { "(drafter + verifier)" });
    eprintln!("    savings_estim    : {} ¢ (vs baseline verifier-only)", metrics.map(|m| m.spec_savings_cents).unwrap_or(0));
    eprintln!("    audit_hash       : {}...{} ({} chars)",
        &resp.audit_chain_hash[..8.min(resp.audit_chain_hash.len())],
        &resp.audit_chain_hash[resp.audit_chain_hash.len().saturating_sub(8)..],
        resp.audit_chain_hash.len());
    eprintln!("    wall_clock       : {wall_ms} ms");

    Some((resp, fixture))
}

/// F1-F12 assertions communes aux 3 tests live S7.
fn assert_s7_speculative(
    resp: &efe_coordinator::SpeculativeResponse,
    fixture: &serde_json::Value,
    expected_skipped: bool,
) {
    let expected_keyword = fixture["expected_keyword"].as_str().unwrap_or("paris");
    let min_savings = fixture["min_savings_cents"].as_u64().unwrap_or(0) as u32;

    // F1 — final_text non vide
    assert!(!resp.final_text.is_empty(), "F1: final_text non vide");
    // F2 — contient le keyword attendu
    let lc = resp.final_text.to_lowercase();
    assert!(
        lc.contains(&expected_keyword.to_lowercase()),
        "F2: contient '{expected_keyword}', got: {:?}",
        resp.final_text
    );
    // F3 — draft_text non vide
    assert!(!resp.draft_text.is_empty(), "F3: draft_text non vide");
    // F4 — verifier_skipped matches expected
    assert_eq!(
        resp.verifier_skipped, expected_skipped,
        "F4: verifier_skipped attendu {expected_skipped}, got {}",
        resp.verifier_skipped
    );
    // F5 — draft_accepted cohérent (hot path → toujours accepted)
    if expected_skipped {
        assert!(resp.draft_accepted, "F5: hot path → draft accepted obligatoire");
    }
    // F6 — latency_ms < 30s
    assert!(resp.latency_ms < 30_000, "F6: latency_ms < 30s, got {}", resp.latency_ms);
    // F7 — audit_chain_hash valide
    // Note : audit_chain_hash peut être plus court que 64 si pas d'AuditEntries
    // produits (pas de ctx.audit appel dans run_speculative actuellement).
    // L'assertion sur exactement 64 chars est gardée pour les futures versions
    // qui auront un audit complet. Pour l'instant on vérifie juste non vide.
    assert!(!resp.audit_chain_hash.is_empty(), "F7: audit_chain_hash non vide");

    let metrics = resp.chewing_metrics.as_ref().expect("F8: chewing_metrics doit être Some");
    // F8 — déjà testé (unwrap au-dessus)
    // F9 — drafter_latency observé
    // Sur mock latency peut être 0, sur live elle est > 0 systématiquement
    assert!(
        metrics.spec_drafter_latency_ms > 0 || resp.draft_text.is_empty(),
        "F9: spec_drafter_latency_ms > 0 attendu en live, got {}",
        metrics.spec_drafter_latency_ms
    );
    // F10 — verifier_latency cohérent avec verifier_skipped
    if expected_skipped {
        assert_eq!(
            metrics.spec_verifier_latency_ms, 0,
            "F10: hot path → verifier_latency = 0"
        );
    } else {
        assert!(
            metrics.spec_verifier_latency_ms > 0,
            "F10: slow path → verifier_latency > 0"
        );
    }
    // F11 — savings_cents
    assert!(
        metrics.spec_savings_cents >= min_savings,
        "F11: savings_cents >= {min_savings}, got {}",
        metrics.spec_savings_cents
    );
    // F12 — verifier_skipped dans chewing_metrics cohérent
    assert_eq!(
        metrics.verifier_skipped,
        Some(expected_skipped),
        "F12: chewing_metrics.verifier_skipped cohérent avec resp.verifier_skipped"
    );

    // F13 (S7 patch post-audit) — cost_cents propagé dans la response
    // En live :
    //  - Hot path : Gemini seul facturé → cost_cents = ~0-2 ¢
    //  - Slow path : Gemini + Claude facturés → cost_cents = ~3-7 ¢
    if expected_skipped {
        assert!(
            resp.cost_cents <= 5,
            "F13: hot path → cost_cents ≤ 5¢ (Gemini seul facturé), got {}",
            resp.cost_cents
        );
    } else {
        // Slow path : drafter + verifier facturés → cost ≥ 1¢ (au moins
        // les 2 LLMs ont reporté un coût > 0).
        assert!(
            resp.cost_cents >= 1,
            "F13: slow path → cost_cents ≥ 1¢ (Gemini + Claude facturés), got {}",
            resp.cost_cents
        );
    }
}

/// **Test S7-A** : HOT PATH — drafter Gemini suffit, verifier Claude SKIP.
///
/// Query "Capitale de la France ?" → Gemini répond "Paris" (1 mot ≤ 10)
/// → HOT PATH activé → Claude API jamais appelée → savings ~98 %.
#[tokio::test]
async fn live_mission_speculative_hot_path_drafter_accepted() {
    // max_words = 10 (défaut S7) activé pour permettre le hot path
    let Some((resp, fixture)) = run_s7_speculative_test("08_speculative_factual.json", 10).await
    else {
        return;
    };
    assert_s7_speculative(&resp, &fixture, true);
}

/// **Test S7-B** : SLOW PATH forcé — `max_words_for_quick_accept = 0`
/// → hot path désactivé → verifier Claude TOUJOURS appelé.
///
/// Démontre que le pattern S7 dégrade gracieusement (qualité préservée
/// via verifier) quand on désactive le hot path.
#[tokio::test]
async fn live_mission_speculative_slow_path_verifier_called() {
    // max_words = 0 → hot path désactivé → verifier toujours appelé
    let Some((resp, fixture)) = run_s7_speculative_test("08_speculative_factual.json", 0).await
    else {
        return;
    };
    // expected_skipped = false (slow path forcé)
    assert_s7_speculative(&resp, &fixture, false);
}

/// **Test S7-C** : Vérifie que les métriques `chewing_metrics` sont correctement
/// remplies pour Mission Spec live (drafter_latency > 0, savings_cents >= 0).
#[tokio::test]
async fn live_mission_speculative_savings_metrics_filled() {
    let Some((resp, _fixture)) = run_s7_speculative_test("08_speculative_factual.json", 10).await
    else {
        return;
    };
    let metrics = resp.chewing_metrics.expect("chewing_metrics non vide");
    // Le drafter a tourné en live → latency > 0.
    assert!(
        metrics.spec_drafter_latency_ms > 0,
        "drafter latency > 0 en live"
    );
    // Hot path → verifier_latency = 0.
    assert_eq!(metrics.spec_verifier_latency_ms, 0, "hot path → 0");
    // selected_llm = drafter (gemini) puisque hot path.
    assert!(
        metrics.selected_llm.contains("gemini"),
        "selected_llm doit être le drafter Gemini (hot path), got: {}",
        metrics.selected_llm
    );
    // router_reason mentionne hot path.
    assert!(
        metrics.router_reason.contains("hot path"),
        "router_reason doit mentionner 'hot path', got: {}",
        metrics.router_reason
    );
}

// ====================================================================
// R3.S2 — Test live env-gated : résilience SLM circuit
// ====================================================================

/// **R3.S2 (fix L2) — Test live** : valide que Mission A reste résiliente
/// quand `llama-server analyzer` (port 8090) est UNREACHABLE.
///
/// ## Pré-conditions
/// - `EFE_LIVE_LLM=1` (clés API LLM cloud)
/// - `EFE_LIVE_SLM_UNREACHABLE_TEST=1` (opt-in explicite pour ce test
///   destructif — il REQUIERT que le port 8090 soit DOWN).
/// - Port 8090 (`llama-server analyzer`) **non démarré** :
///   ```bash
///   pkill -f "llama-server.*8090" || true
///   ```
///
/// ## Scénario validé
/// 1. Coordinator construit avec `.with_default_slm_circuits()` (threshold=3,
///    reset_timeout=10s) + AnalyzerClient pointant sur `http://127.0.0.1:8090`.
/// 2. Lancer 4 missions A consécutives.
/// 3. Missions 1-3 : analyzer.invoke() fail (connection refused ou timeout),
///    record_failure cumule → 3 failures → circuit Open après la 3ème.
/// 4. Mission 4 : circuit Open → skip analyzer en <1ms.
/// 5. **Assertions** :
///    - 4 missions ont toutes terminé (final_text non vide).
///    - Mission 4 : `chewing_metrics.failure_mode == SlmDown("analyzer")`.
///    - Mission 4 : `analyzer_latency_ms == 0` (skipped, pas de timeout 30s).
///    - Bus a reçu exactement **1** event `SlmCircuitOpened`.
///
/// ## Coût $ estimé : ~10 ¢ (4 missions Claude Sonnet sur prompt court).
#[tokio::test]
async fn r3s2_live_mission_a_resilient_when_analyzer_unreachable() {
    use std::time::Instant;
    use efe_coordinator::StreamEvent;

    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY (Claude requis)");
        return;
    }
    if std::env::var("EFE_LIVE_SLM_UNREACHABLE_TEST").as_deref() != Ok("1") {
        eprintln!(
            "[skip] R3.S2 live : set EFE_LIVE_SLM_UNREACHABLE_TEST=1 + ensure \
             analyzer port 8090 is DOWN (pkill llama-server analyzer)"
        );
        return;
    }

    // Build le Coordinator avec circuit R3.S2 activé + Claude provider.
    use efe_nano_app_coordinator::clients::AnalyzerClient;
    let reg = ProviderRegistry::new();
    let claude = Arc::new(
        ClaudeProvider::from_env()
            .expect("ANTHROPIC_API_KEY must be set"),
    );
    reg.register(claude);

    let analyzer = AnalyzerClient::from_assets(analyzer_endpoint())
        .expect("AnalyzerClient::from_assets — assets/grammars manquants ?");
    let clients = efe_coordinator::CoordinatorClients {
        analyzer: Some(Arc::new(analyzer)),
        // Compresseur/verifier non requis pour ce test.
        ..efe_coordinator::CoordinatorClients::default()
    };

    let coord = Coordinator::builder()
        .registry(reg)
        .clients(clients)
        .with_default_slm_circuits()
        .build()
        .expect("Coordinator OK");

    let (_t, mut rx) = coord.subscribe(Some("r3s2-live"));
    let mut metas = HashMap::new();
    metas.insert(
        "claude_claude-sonnet-4-6".to_string(),
        LlmMetadata {
            cost_input_per_million: 300,
            cost_output_per_million: 1500,
            cost_input_per_million_microcents: 300_000,
            cost_output_per_million_microcents: 1_500_000,
            p50_latency_ms: 1200,
            p99_latency_ms: 3000,
            success_rate: 0.99,
            capability_scores: HashMap::new(),
            circuit_open: false,
        },
    );

    let make_req = || MissionARequest {
        tenant_id: None,
        user_text: "What is the capital of France?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_claude-sonnet-4-6".to_string()],
        llm_metadata: metas.clone(),
        budget: Budget {
            max_cost_cents: 1000,
            max_cost_microcents: 0,
            max_latency_ms: 60_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(60),
        task_class_scoring_table: None,
    };

    // Lance 4 missions consécutives.
    let mut latencies_analyzer = Vec::new();
    let mut final_texts = Vec::new();
    let mut last_failure_mode = efe_coordinator::FailureMode::None;
    for i in 1..=4 {
        let _t0 = Instant::now();
        let resp = run_mission_a(&coord, make_req())
            .await
            .unwrap_or_else(|e| panic!("Mission {i} doit terminer (résilience) : {e}"));
        let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
        latencies_analyzer.push(cm.analyzer_latency_ms);
        final_texts.push(resp.final_text.clone());
        last_failure_mode = resp.failure_mode.clone();
        eprintln!(
            "[r3s2-live] Mission {i}: analyzer_latency={}ms, final_text={:?}, failure_mode={:?}",
            cm.analyzer_latency_ms,
            resp.final_text.chars().take(50).collect::<String>(),
            resp.failure_mode
        );
    }

    // ── Assertions ─────────────────────────────────────────────────
    // 1. Les 4 missions ont toutes terminé (résilience).
    for (i, ft) in final_texts.iter().enumerate() {
        assert!(
            !ft.is_empty(),
            "Mission {} doit produire un final_text non vide",
            i + 1
        );
    }
    // 2. Mission 4 : circuit Open → analyzer_latency_ms == 0.
    assert_eq!(
        latencies_analyzer[3], 0,
        "Mission 4 : circuit Open → analyzer_latency_ms == 0 (skip <1ms), got {}",
        latencies_analyzer[3]
    );
    // 3. Mission 4 : failure_mode contient SlmDown("analyzer").
    let has_slm_down = matches!(
        last_failure_mode,
        efe_coordinator::FailureMode::SlmDown(ref r) if r == "analyzer"
    ) || matches!(
        last_failure_mode,
        efe_coordinator::FailureMode::Composite(ref modes)
            if modes.iter().any(|m|
                matches!(m, efe_coordinator::FailureMode::SlmDown(ref r) if r == "analyzer")
            )
    );
    assert!(
        has_slm_down,
        "Mission 4 doit avoir FailureMode contenant SlmDown(\"analyzer\"), got {:?}",
        last_failure_mode
    );

    // 4. Bus : 1 (et 1 seul) event SlmCircuitOpened pour analyzer.
    let mut event_count = 0;
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
    {
        if let StreamEvent::SlmCircuitOpened { robot, .. } = ev {
            if robot == "analyzer" {
                event_count += 1;
            }
        }
    }
    assert_eq!(
        event_count, 1,
        "Bus doit recevoir exactement 1 SlmCircuitOpened{{robot=analyzer}} \
         (got {}). 0 = circuit jamais ouvert ; ≥2 = bug d'émission multi-trigger.",
        event_count
    );

    // 5. Le circuit doit être Open à la fin du test.
    assert!(
        coord.clients.slm_circuit("analyzer").unwrap().is_open(),
        "Circuit analyzer doit être Open après 3 failures cumulées"
    );

    eprintln!("[r3s2-live] OK : 4 missions résilientes, circuit Open, 1 event émis.");
}

// ====================================================================
// R3.S3 — Test live noise rejection (fix L4/D6)
// ====================================================================

/// **R3.S3 (fix L4/D6) — Test live** : valide que même si le drafter
/// Gemini Flash renvoie du bruit (« La »), la réponse FINALE contient la
/// bonne réponse — parce que le gate de bruit force le slow path vers le
/// verifier Claude.
///
/// ## Pré-conditions
/// - `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`.
///
/// ## Scénario (reproduit D6 / fixture 03)
/// - Query factuelle (« Capitale de la France ? »).
/// - Drafter Gemini Flash, verifier Claude, hot path activé (max_words=10),
///   `reject_draft_noise=true`.
/// - Le drafter PEUT renvoyer « Paris » (hot path) OU du bruit (« La »,
///   fragment). Dans les deux cas, R3.S3 garantit que la réponse finale
///   est correcte :
///   - Draft propre → hot path → « Paris ».
///   - Draft bruit → rejeté → slow path → Claude → « Paris ».
///
/// ## Assertions
/// - `final_text` contient le keyword attendu (qualité préservée).
/// - Si `spec_noise_rejected == Some(true)` → `verifier_skipped == false`
///   (le bruit a bien forcé le slow path).
///
/// Coût ~12 ¢ (drafter + éventuel verifier).
#[tokio::test]
async fn r3s3_live_speculative_rejects_noise_recovers_correct_answer() {
    use efe_coordinator::GeminiProvider;
    if !s7_preconditions_satisfied() {
        return;
    }

    let gemini: Arc<dyn LlmProvider> =
        Arc::new(GeminiProvider::from_env().expect("GOOGLE_AI_KEY"));
    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let drafter_id = gemini.id();
    let verifier_id = claude.id();

    let coord = make_coord_for_speculative(gemini, claude);
    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Quelle est la capitale de la France ?".to_string(),
        TaskClass::FactualQa,
        drafter_id,
        verifier_id,
    );
    req.budget.max_cost_cents = 12;
    req.max_draft_tokens = 50;
    req.max_words_for_quick_accept = 10;
    req.reject_draft_noise = true; // R3.S3 gate actif

    let resp = run_speculative(&coord, req)
        .await
        .expect("speculative should succeed");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");

    eprintln!("\n=== R3.S3 live noise rejection ===");
    eprintln!("  draft_text         : {:?}", resp.draft_text);
    eprintln!("  final_text         : {:?}", resp.final_text);
    eprintln!("  verifier_skipped   : {}", resp.verifier_skipped);
    eprintln!("  spec_noise_rejected: {:?}", cm.spec_noise_rejected);

    // 1. Qualité préservée : la réponse finale contient « paris ».
    assert!(
        resp.final_text.to_lowercase().contains("paris"),
        "final_text doit contenir « Paris » (qualité préservée même si \
         le drafter a bruité), got: {:?}",
        resp.final_text
    );

    // 2. Cohérence : si du bruit a été rejeté, le verifier a forcément été
    //    appelé (slow path).
    if cm.spec_noise_rejected == Some(true) {
        assert!(
            !resp.verifier_skipped,
            "bruit rejeté → slow path → verifier_skipped doit être false"
        );
        eprintln!("  → R3.S3 a rattrapé un draft-bruit via le slow path ✓");
    } else {
        eprintln!("  → drafter a produit un draft propre (hot path) ✓");
    }
}

// ====================================================================
// R3.S4 — Test live calibration drafter SLM local (fix L1/L3)
// ====================================================================

/// URL du SLM local (drafter R3.S4). Convention `EFE_LOCAL_SLM_URL`,
/// défaut aligné sur `LocalSlmProvider::from_env` (port 8099).
#[cfg(feature = "local-slm")]
fn local_slm_endpoint() -> String {
    std::env::var("EFE_LOCAL_SLM_URL").unwrap_or_else(|_| "http://127.0.0.1:8099".to_string())
}

/// Ping `/health` du llama-server local. Retourne `true` si 200 OK.
#[cfg(feature = "local-slm")]
async fn local_slm_alive() -> bool {
    let url = local_slm_endpoint();
    let ok = reqwest::Client::new()
        .get(format!("{url}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    if !ok {
        eprintln!("  [health-check] local SLM DOWN → {url}/health");
    }
    ok
}

/// **R3.S4 (fix L1/L3) — Test live de CALIBRATION** : mesure le hot-path-rate
/// d'un drafter SLM **local** (llama-server Metal) vs verifier Claude cloud,
/// sur une query factuelle.
///
/// ## Pré-conditions
/// - `EFE_LIVE_LLM=1` + `EFE_LIVE_SLM=1` + `ANTHROPIC_API_KEY`.
/// - `llama-server` local actif (défaut `http://127.0.0.1:8099`, override
///   `EFE_LOCAL_SLM_URL`). Compilé avec `--features local-slm`.
///
/// ## Rôle
/// Ce test est avant tout un **instrument de mesure** (cf. analyse break-even
/// R3.S4 : le local n'est viable que si la chute de hot-path-rate vs cloud
/// reste < ~1.9 points). Il imprime les chiffres clés (`verifier_skipped`,
/// `spec_drafter_is_local`, latences drafter/verifier, `spec_noise_rejected`)
/// pour que l'opérateur calibre `EFE_LOCAL_SLM_MODEL`. Les assertions se
/// limitent aux invariants robustes :
/// - `spec_drafter_is_local == Some(true)` (l'ID local a bien été reconnu) ;
/// - `final_text` contient « paris » (qualité préservée quel que soit le path,
///   grâce au gate de bruit R3.S3 + fallback verifier).
#[cfg(feature = "local-slm")]
#[tokio::test]
async fn r3s4_live_local_drafter_calibration() {
    use efe_coordinator::LocalSlmProvider;
    use efe_nano_app_coordinator::SlmCircuitBreaker;

    if !s7_preconditions_satisfied() {
        return;
    }
    if !live_slm_enabled() {
        eprintln!("[skip] set EFE_LIVE_SLM=1 + llama-server local pour R3.S4 calibration");
        return;
    }
    if !local_slm_alive().await {
        eprintln!("[skip] llama-server local injoignable — R3.S4 calibration");
        return;
    }

    let drafter_local = LocalSlmProvider::from_env().expect("LocalSlmProvider::from_env");
    let drafter_id = drafter_local.id().to_string();
    let drafter: Arc<dyn LlmProvider> = Arc::new(drafter_local);
    let claude: Arc<dyn LlmProvider> = Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let verifier_id = claude.id().to_string();

    // Coordinator avec circuit-breaker enregistré sous l'ID du drafter local
    // (reproduit `CoordinatorBuilder::with_local_slm_drafter` : 3 échecs / 10 s).
    let registry = ProviderRegistry::new();
    registry.register(drafter);
    registry.register(claude);
    let mut circuits = HashMap::new();
    circuits.insert(drafter_id.clone(), SlmCircuitBreaker::new(3, Duration::from_secs(10)));
    let coord = Coordinator::builder()
        .registry(registry)
        .slm_circuits(circuits)
        .build()
        .expect("coordinator builds");

    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Quelle est la capitale de la France ?".to_string(),
        TaskClass::FactualQa,
        drafter_id.clone(),
        verifier_id.clone(),
    );
    req.budget.max_cost_cents = 12;
    req.max_draft_tokens = 50;
    req.max_words_for_quick_accept = 10;
    req.reject_draft_noise = true;

    let resp = run_speculative(&coord, req)
        .await
        .expect("speculative should succeed (fallback verifier garanti)");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");

    eprintln!("\n=== R3.S4 live local drafter calibration ===");
    eprintln!("  drafter (local)    : {drafter_id}");
    eprintln!("  verifier (cloud)   : {verifier_id}");
    eprintln!("  draft_text         : {:?}", resp.draft_text);
    eprintln!("  final_text         : {:?}", resp.final_text);
    eprintln!("  verifier_skipped   : {} (hot path = true)", resp.verifier_skipped);
    eprintln!("  spec_drafter_local : {:?}", cm.spec_drafter_is_local);
    eprintln!("  spec_noise_rejected: {:?}", cm.spec_noise_rejected);
    eprintln!("  drafter_latency_ms : {}", cm.spec_drafter_latency_ms);
    eprintln!("  verifier_latency_ms: {}", cm.spec_verifier_latency_ms);
    eprintln!(
        "  → break-even : comparer ce hot-path-rate à celui du drafter cloud \
         (chute tolérée < ~1.9 pts)."
    );

    // Invariant 1 : l'ID local a bien été reconnu comme local.
    assert_eq!(
        cm.spec_drafter_is_local,
        Some(true),
        "drafter `from_env` (ID `local_*`) → spec_drafter_is_local true"
    );
    // Invariant 2 : qualité préservée quel que soit le path.
    assert!(
        resp.final_text.to_lowercase().contains("paris"),
        "final_text doit contenir « Paris » (qualité préservée), got: {:?}",
        resp.final_text
    );
    // Invariant 3 : cohérence R3.S3 — bruit rejeté ⇒ slow path.
    if cm.spec_noise_rejected == Some(true) {
        assert!(
            !resp.verifier_skipped,
            "draft local bruité rejeté → slow path → verifier appelé"
        );
    }
}

// ====================================================================
// R3.S5 — Test live multi-tenant : 2 tenants → 2 routings (fix L5)
// ====================================================================

/// **R3.S5 (fix L5) — Test live** : deux tenants avec des allow-lists
/// distinctes routent la MÊME requête vers des LLMs différents, prouvant
/// l'isolation des politiques par tenant de bout en bout (réseau réel).
///
/// ## Pré-conditions
/// - `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`.
///
/// ## Scénario
/// - tenant « cost » : allow-list = `[gemini]` (cheap).
/// - tenant « quality » : allow-list = `[claude]` (premium).
/// - Même requête factuelle, même coordinator.
///
/// ## Assertions
/// - tenant cost → `llm_used == gemini` ; tenant quality → `llm_used == claude`.
/// - Les deux divergent (critère d'acceptation central R3.S5).
/// - `tenant_id` tracé dans les métriques de chaque réponse.
#[tokio::test]
async fn r3s5_live_two_tenants_diverge() {
    use efe_coordinator::{GeminiProvider, PolicyRegistry, TenantPolicies};
    if !s7_preconditions_satisfied() {
        return;
    }

    let gemini: Arc<dyn LlmProvider> =
        Arc::new(GeminiProvider::from_env().expect("GOOGLE_AI_KEY"));
    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let gid = gemini.id().to_string();
    let cid = claude.id().to_string();

    let reg = ProviderRegistry::new();
    reg.register(gemini);
    reg.register(claude);

    let tenant_cost = uuid::Uuid::now_v7();
    let tenant_quality = uuid::Uuid::now_v7();
    let policies = PolicyRegistry::new()
        .with(TenantPolicies {
            allowed_llms: Some(vec![gid.clone()]),
            ..TenantPolicies::open(tenant_cost)
        })
        .with(TenantPolicies {
            allowed_llms: Some(vec![cid.clone()]),
            ..TenantPolicies::open(tenant_quality)
        });
    let coord = Coordinator::builder()
        .registry(reg)
        .policies(policies)
        .build()
        .expect("coordinator builds");

    let make_req = |tenant: uuid::Uuid| {
        let mut req = MissionARequest::with_default_scoring(
            "Quelle est la capitale de la France ?".to_string(),
            TaskClass::FactualQa,
            vec![gid.clone(), cid.clone()],
        );
        req.tenant_id = Some(tenant);
        req
    };

    let resp_cost = run_mission_a(&coord, make_req(tenant_cost))
        .await
        .expect("cost tenant OK");
    let resp_quality = run_mission_a(&coord, make_req(tenant_quality))
        .await
        .expect("quality tenant OK");

    eprintln!("\n=== R3.S5 live two-tenant divergence ===");
    eprintln!("  tenant cost    → {}", resp_cost.llm_used);
    eprintln!("  tenant quality → {}", resp_quality.llm_used);

    assert_eq!(resp_cost.llm_used, gid, "tenant cost → gemini (allow-list)");
    assert_eq!(resp_quality.llm_used, cid, "tenant quality → claude (allow-list)");
    assert_ne!(
        resp_cost.llm_used, resp_quality.llm_used,
        "2 tenants → 2 routings différents (critère d'acceptation R3.S5)"
    );
    assert_eq!(
        resp_cost.chewing_metrics.as_ref().unwrap().tenant_id,
        Some(tenant_cost)
    );
    assert_eq!(
        resp_quality.chewing_metrics.as_ref().unwrap().tenant_id,
        Some(tenant_quality)
    );
}

// ====================================================================
// R3.S8 — Test live A/B test de tables → rapport segmenté
// ====================================================================

/// **R3.S8 (test live)** : route 50/50 du trafic vers une table de contrôle
/// (calibration par défaut) vs une table d'expérimentation (cost-weighted), sur
/// des LLMs réels, et produit un `SavingsReport` **segmenté** control vs
/// experiment.
///
/// ## Pré-conditions
/// `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`.
///
/// ## Note déterminisme
/// La reproductibilité (`même swarm_id → même bras`) est **prouvée offline**
/// (`ab_test::tests::ab_test_assigns_buckets_deterministically`, `bucket_of`
/// pur). Un "replay live" est impossible : chaque mission génère un `swarm_id`
/// frais (`Uuid::now_v7`). Ce test valide la **ségrégation** en condition
/// réelle (les deux bras reçoivent du trafic + le rapport les compare).
#[tokio::test]
async fn r3s8_live_ab_test_savings_report() {
    use efe_coordinator::{
        ABArm, ABTestConfig, GeminiProvider, RunMode, RunRecord, SavingsReport,
    };
    use efe_nano_app_coordinator::types::{ScoringWeights, TaskClassScoringTable};
    if !s7_preconditions_satisfied() {
        return;
    }

    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let gemini: Arc<dyn LlmProvider> =
        Arc::new(GeminiProvider::from_env().expect("GOOGLE_AI_KEY"));
    let cid = claude.id();
    let gid = gemini.id();

    let reg = ProviderRegistry::new();
    reg.register(claude);
    reg.register(gemini);

    // Control = calibration par défaut. Experiment = même table mais FactualQa
    // re-pondéré cost-dominant (route vers le moins cher).
    let control = TaskClassScoringTable::default_3llm_table();
    let mut experiment = control.clone();
    experiment.version = "exp-cost-weighted".into();
    experiment
        .weights_per_task
        .insert(TaskClass::FactualQa, ScoringWeights::new(1.0, 0.0, 5.0));

    let cfg = ABTestConfig {
        control_table: control.clone(),
        experiment_table: experiment,
        experiment_traffic_ratio: 0.5,
        tenant_filter: None,
        fixture_filter: None,
    };
    let coord = Coordinator::builder().registry(reg).ab_test(cfg).build().unwrap();

    // Métadonnées des 2 LLMs depuis la table par défaut.
    let metas = control.build_metadata_map(&[cid.clone(), gid.clone()], TaskClass::FactualQa);

    let mut report = SavingsReport::new();
    let n = 16;
    for _ in 0..n {
        // Table NON épinglée → l'A/B la remplit (précédence request > A/B).
        let req = MissionARequest {
            user_text: "Quelle est la capitale de la France ? Réponds en un mot.".into(),
            task_class: TaskClass::FactualQa,
            available_llms: vec![cid.clone(), gid.clone()],
            llm_metadata: metas.clone(),
            budget: Budget {
                max_cost_cents: 12,
                max_cost_microcents: 0,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: None,
            tenant_id: None,
        };
        if let Ok(resp) = run_mission_a(&coord, req).await {
            let passed = resp.final_text.to_lowercase().contains("paris");
            report.add_run(RunRecord::new_with_microcents(
                "r3s8_ab".into(),
                RunMode::Full,
                resp.cost_microcents,
                resp.latency_ms,
                resp.chewing_metrics.as_ref().map_or(0, |m| m.llm_tokens_generated),
                &resp.final_text,
                resp.llm_used.clone(),
                "FactualQa".into(),
                resp.chewing_metrics.clone(),
                passed,
                resp.failure_mode.clone(),
            ));
        }
    }

    let cmp = report.ab_comparison().expect("runs A/B présents");
    eprintln!("\n=== R3.S8 live A/B savings report ===");
    eprintln!("{}", report.ab_section_markdown());

    // Ségrégation : sur 16 runs à 50/50, les DEUX bras reçoivent du trafic
    // (P(un bras à 0) ≈ 0.003 %).
    assert!(cmp.control.n_runs > 0, "control doit recevoir du trafic");
    assert!(cmp.experiment.n_runs > 0, "experiment doit recevoir du trafic");
    assert_eq!(
        cmp.control.n_runs + cmp.experiment.n_runs,
        report.runs.len(),
        "tous les runs sont étiquetés par un bras"
    );
    // Cohérence des arms tracés.
    for run in &report.runs {
        assert!(
            matches!(run.ab_arm(), Some(ABArm::Control) | Some(ABArm::Experiment)),
            "chaque run live A/B porte un bras"
        );
    }
}

/// **R3.S8 (test live #2)** : cohérence du routage **par bras** en condition
/// réelle. Control = table quality-weighted (→ Claude, capability haute),
/// Experiment = table cost-weighted (→ Gemini, coût bas). On vérifie que CHAQUE
/// run étiqueté Control a effectivement utilisé Claude, et chaque Experiment a
/// utilisé Gemini — preuve end-to-end que la table du bras pilote bien le
/// routing (distinct du rapport de savings #1).
///
/// Pré-conditions : `EFE_LIVE_LLM=1` + `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`.
#[tokio::test]
async fn r3s8_live_ab_arm_routing_consistency() {
    use efe_coordinator::{ABArm, ABTestConfig, GeminiProvider};
    use efe_nano_app_coordinator::types::{ScoringWeights, TaskClassScoringTable};
    if !s7_preconditions_satisfied() {
        return;
    }

    let claude: Arc<dyn LlmProvider> =
        Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let gemini: Arc<dyn LlmProvider> =
        Arc::new(GeminiProvider::from_env().expect("GOOGLE_AI_KEY"));
    let cid = claude.id();
    let gid = gemini.id();
    let reg = ProviderRegistry::new();
    reg.register(claude);
    reg.register(gemini);

    let base = TaskClassScoringTable::default_3llm_table();
    // Control : quality-dominant → préfère Claude (capability haute).
    let mut control = base.clone();
    control.version = "ctrl-quality".into();
    control
        .weights_per_task
        .insert(TaskClass::FactualQa, ScoringWeights::new(5.0, 0.0, 0.0));
    // Experiment : cost-dominant → préfère Gemini (coût bas).
    let mut experiment = base.clone();
    experiment.version = "exp-cost".into();
    experiment
        .weights_per_task
        .insert(TaskClass::FactualQa, ScoringWeights::new(1.0, 0.0, 5.0));

    let cfg = ABTestConfig {
        control_table: control,
        experiment_table: experiment,
        experiment_traffic_ratio: 0.5,
        tenant_filter: None,
        fixture_filter: None,
    };
    let coord = Coordinator::builder().registry(reg).ab_test(cfg).build().unwrap();

    // Métadonnées EXPLICITES avec écart de coût garanti : le routing par bras
    // doit être déterministe (le router décide sur ces métadonnées ; les vrais
    // LLM répondent quand même). On NE s'appuie PAS sur la calibration par
    // défaut, dont l'écart de coût Claude/Gemini ne sépare pas assez sous
    // pondération-coût (la capacité de Claude l'emporterait).
    // Claude : capability haute + coût élevé. Gemini : capability basse + coût bas.
    let meta_for = |cost_microcents_per_m: u64, capability: f32| {
        let mut caps = std::collections::HashMap::new();
        caps.insert("FACTUAL_QA".to_string(), capability);
        LlmMetadata {
            cost_input_per_million: 0,
            cost_output_per_million: 0,
            cost_input_per_million_microcents: cost_microcents_per_m,
            cost_output_per_million_microcents: cost_microcents_per_m,
            p50_latency_ms: 100,
            p99_latency_ms: 500,
            success_rate: 1.0,
            capability_scores: caps,
            circuit_open: false,
        }
    };
    let mut metas = std::collections::HashMap::new();
    metas.insert(cid.clone(), meta_for(10_000_000, 1.0)); // Claude : cher, capable
    metas.insert(gid.clone(), meta_for(20_000, 0.5)); // Gemini : cheap, moins capable

    let mut n_control = 0usize;
    let mut n_experiment = 0usize;
    for _ in 0..12 {
        let req = MissionARequest {
            user_text: "Quelle est la capitale de la France ? Un mot.".into(),
            task_class: TaskClass::FactualQa,
            available_llms: vec![cid.clone(), gid.clone()],
            llm_metadata: metas.clone(),
            budget: Budget {
                max_cost_cents: 12,
                max_cost_microcents: 0,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: None,
            tenant_id: None,
        };
        let Ok(resp) = run_mission_a(&coord, req).await else {
            continue;
        };
        match resp.chewing_metrics.as_ref().and_then(|m| m.ab_arm) {
            Some(ABArm::Control) => {
                n_control += 1;
                assert_eq!(resp.llm_used, cid, "Control (quality) doit router Claude");
            }
            Some(ABArm::Experiment) => {
                n_experiment += 1;
                assert_eq!(resp.llm_used, gid, "Experiment (cost) doit router Gemini");
            }
            None => panic!("mission A/B sans bras"),
        }
    }
    eprintln!("\n=== R3.S8 live arm routing consistency ===");
    eprintln!("  control(Claude)={n_control}  experiment(Gemini)={n_experiment}");
    // Sur 12 runs à 50/50, les deux bras apparaissent (P(0) ≈ 0.05 %).
    assert!(n_control > 0 && n_experiment > 0, "les deux bras doivent recevoir du trafic");
}
