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
use efe_coordinator::workflows::run_mission_a;
use efe_coordinator::{
    ClaudeProvider, Coordinator, MissionARequest, MissionBRequest,
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
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            // Cap budget : 10 ¢ = ~3 000 tokens cumulés Sonnet (large pour fixture).
            max_cost_cents: 10,
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
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            // Budget élargi vs S2 car analyzer + verifier ajoutent ~1 s cumulés.
            max_cost_cents: 10,
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
