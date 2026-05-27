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
use efe_coordinator::{ClaudeProvider, Coordinator, MissionARequest};
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
