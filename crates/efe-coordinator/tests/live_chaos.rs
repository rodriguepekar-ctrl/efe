// SPDX-License-Identifier: UNLICENSED
//! **R3.S6 — Tests live de chaos engineering (fix L3).**
//!
//! Injectent de **vraies** fautes (429/timeout/réponse dégénérée) sur des
//! providers cloud RÉELS et prouvent que la graceful degradation S8-R2 tient
//! en condition réseau réelle — pas seulement contre `MockLlmProvider`.
//!
//! ## Pré-conditions
//! - `EFE_LIVE_LLM=1`
//! - `ANTHROPIC_API_KEY` + `GOOGLE_AI_KEY`
//! - build avec `--features "chaos,all-providers"`
//!
//! Sans ces variables, chaque test fait un early-return en `[skip]`.
//! Exécutés en CI nightly via `.github/workflows/chaos-eng.yml`.
//!
//! Gate de compilation : nécessite `chaos` + `anthropic` + `gemini`.

#![cfg(all(feature = "chaos", feature = "anthropic", feature = "gemini"))]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::failure::FailureMode;
use efe_coordinator::llm::LlmProvider;
use efe_coordinator::{
    run_mission_a, run_mission_b, run_speculative, ChaosFaultKind, ChaosInjector, ClaudeProvider,
    Coordinator, CoordinatorClients, GeminiProvider, MissionARequest, MissionBRequest,
    ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

/// `true` si `EFE_LIVE_LLM=1` + clés Claude & Gemini présentes.
fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_LLM").as_deref() == Ok("1")
        && std::env::var("ANTHROPIC_API_KEY").is_ok()
        && std::env::var("GOOGLE_AI_KEY").is_ok()
}

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 800,
        p99_latency_ms: 5000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

fn budget() -> Budget {
    Budget {
        max_cost_cents: 20,
        max_cost_microcents: 0,
        max_latency_ms: 30_000,
        min_quality_score: 0.0,
    }
}

/// Borne dure par mission (le workflow n'enforce pas de hard-deadline — cf.
/// architecture §11.2 ; on protège la CI ici, côté harness).
const MISSION_DEADLINE: Duration = Duration::from_secs(40);

// =====================================================================
// Live #1 — Mission A : 429 Claude à 30 % → fallback Gemini
// =====================================================================

/// 20 missions consécutives avec Claude wrap chaos 429 @ 30 %, Gemini sain en
/// fallback. Asserte : > 80 % complètent, et le chaos a réellement déclenché
/// des fallbacks tracés.
#[tokio::test]
async fn live_chaos_mission_a_429_at_30pct() {
    if !live_enabled() {
        eprintln!("[skip] EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY + GOOGLE_AI_KEY requis");
        return;
    }
    let claude = Arc::new(ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY"));
    let gemini = Arc::new(GeminiProvider::from_env().expect("GOOGLE_AI_KEY"));
    let cid = claude.id();
    let gid = gemini.id();

    // Claude faute selon `EFE_CHAOS_RATE` (CI = 0.3 ; absent ⇒ 0.0 ⇒ no-op).
    // `from_env` câble enfin la variable d'environnement du workflow CI — sans
    // ça, le taux était codé en dur et `EFE_CHAOS_RATE` était du dead config.
    let injector = Arc::new(ChaosInjector::from_env(
        ChaosFaultKind::HttpStatus(429),
        &[cid.clone()],
    ));
    let effective_rate = injector.fault_rate(&cid);
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(claude)); // id = cid, faute 30 %
    reg.register(gemini);
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert(cid.clone(), meta());
    metas.insert(gid.clone(), meta());

    let n = 20usize;
    let mut successes = 0usize;
    let mut fallbacks = 0usize;
    for i in 0..n {
        // Sans table → selected = available.first() = Claude (cible du chaos).
        let req = MissionARequest {
            user_text: "Quelle est la capitale de la France ? Réponds en un mot.".into(),
            task_class: TaskClass::FactualQa,
            available_llms: vec![cid.clone(), gid.clone()],
            llm_metadata: metas.clone(),
            budget: budget(),
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: None,
            tenant_id: None,
        };
        match tokio::time::timeout(MISSION_DEADLINE, run_mission_a(&coord, req)).await {
            Ok(Ok(resp)) => {
                successes += 1;
                if matches!(resp.failure_mode, FailureMode::LlmFallbackTriggered { .. }) {
                    fallbacks += 1;
                }
                assert!(!resp.final_text.is_empty(), "mission #{i} texte vide");
            }
            Ok(Err(e)) => eprintln!("  mission #{i} ERROR: {e}"),
            Err(_) => eprintln!("  mission #{i} TIMEOUT"),
        }
    }
    let rate = successes as f32 / n as f32;
    eprintln!("\n=== R3.S6 live chaos Mission A 429 (EFE_CHAOS_RATE={effective_rate}) ===");
    eprintln!("  successes : {successes}/{n} ({:.0}%)", rate * 100.0);
    eprintln!("  fallbacks : {fallbacks}");
    assert!(rate > 0.80, "résilience : > 80 % attendu, got {:.0}%", rate * 100.0);
    // Le fallback ne peut être prouvé que si une faute a été injectée. Avec
    // `EFE_CHAOS_RATE=0` (var absente), aucune faute → on n'asserte pas.
    if effective_rate > 0.0 {
        assert!(
            fallbacks > 0,
            "EFE_CHAOS_RATE={effective_rate} sur {n} missions aurait dû déclencher ≥ 1 fallback"
        );
    } else {
        eprintln!("  (EFE_CHAOS_RATE absent/0 → aucune faute injectée, assertion fallback ignorée)");
    }
}

// =====================================================================
// Live #2 — Mission B : 1 LLM en timeout → vote dégradé
// =====================================================================

#[tokio::test]
async fn live_chaos_mission_b_one_llm_timeout() {
    if !live_enabled() {
        eprintln!("[skip] live keys requis");
        return;
    }
    let claude = Arc::new(ClaudeProvider::from_env().expect("key"));
    let gemini = Arc::new(GeminiProvider::from_env().expect("key"));
    let cid = claude.id();
    let gid = gemini.id();

    // Claude time out systématiquement (borné CHAOS_MAX_SLEEP) → seul Gemini vote.
    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::Timeout(Duration::from_secs(60)), 1)
            .with_fault_rate(&cid, 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(claude));
    reg.register(gemini);
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert(cid.clone(), meta());
    metas.insert(gid.clone(), meta());
    let req = MissionBRequest {
        user_text: "Capitale de la France ? Un mot.".into(),
        task_class: TaskClass::FactualQa,
        llms: vec![cid.clone(), gid.clone()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(30),
        tenant_id: None,
    };
    let resp = tokio::time::timeout(MISSION_DEADLINE, run_mission_b(&coord, req))
        .await
        .expect("pas de hang")
        .expect("mission B survit avec 1 votant");

    eprintln!("\n=== R3.S6 live chaos Mission B timeout ===");
    eprintln!("  winner: {} | degraded: {} | failed: {:?}", resp.winner_llm, resp.vote_degraded, resp.failed_llms);
    assert!(resp.vote_degraded, "1 LLM timeout → vote dégradé");
    assert_eq!(resp.winner_llm, gid, "Gemini (seul survivant) gagne");
}

// =====================================================================
// Live #3 — Speculative : drafter Gemini réponse invalide → slow path
// =====================================================================

#[tokio::test]
async fn live_chaos_speculative_drafter_invalid_response() {
    if !live_enabled() {
        eprintln!("[skip] live keys requis");
        return;
    }
    let gemini = Arc::new(GeminiProvider::from_env().expect("key")); // drafter
    let claude = Arc::new(ClaudeProvider::from_env().expect("key")); // verifier
    let gid = gemini.id();
    let cid = claude.id();

    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::InvalidResponse, 2).with_fault_rate(&gid, 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(gemini)); // drafter renvoie un draft invalide
    reg.register(claude);
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Quelle est la capitale de la France ?".into(),
        TaskClass::FactualQa,
        gid.clone(),
        cid.clone(),
    );
    // max_words=1 → tout draft multi-mots (dont le garbage) → slow path →
    // le verifier (Claude) arbitre et produit la vraie réponse.
    req.max_words_for_quick_accept = 1;
    req.reject_draft_noise = true;

    let resp = tokio::time::timeout(MISSION_DEADLINE, run_speculative(&coord, req))
        .await
        .expect("pas de hang")
        .expect("spec survit");

    eprintln!("\n=== R3.S6 live chaos Speculative invalid drafter ===");
    eprintln!("  final: {:?} | verifier_skipped: {}", resp.final_text, resp.verifier_skipped);
    assert!(
        resp.final_text.to_lowercase().contains("paris"),
        "qualité préservée malgré drafter invalide, got {:?}",
        resp.final_text
    );
    assert!(!resp.verifier_skipped, "drafter invalide → slow path");
}

// =====================================================================
// Live #4 — SLM analyzer injoignable → skip gracieux (réutilise R3.S2)
// =====================================================================

/// Vecteur infra-level : on pointe l'analyzer vers un port mort. Le nano-robot
/// (NanoRobotClient, pas un LlmProvider → pas de ChaosLlmProvider) échoue ;
/// la mission continue en mode dégradé (texte brut). Valide la résilience SLM
/// posée en R3.S2 contre une vraie panne réseau.
///
/// Ne nécessite PAS de llama-server : le port mort suffit (gate Claude only).
#[tokio::test]
async fn live_chaos_slm_analyzer_down() {
    if std::env::var("EFE_LIVE_LLM").as_deref() != Ok("1")
        || std::env::var("ANTHROPIC_API_KEY").is_err()
    {
        eprintln!("[skip] EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY requis");
        return;
    }
    use efe_nano_app_coordinator::clients::AnalyzerClient;

    let claude = Arc::new(ClaudeProvider::from_env().expect("key"));
    let cid = claude.id();

    // Analyzer pointé vers un port mort (connexion refusée immédiate).
    let analyzer = Arc::new(
        AnalyzerClient::from_assets("http://127.0.0.1:1")
            .expect("AnalyzerClient::from_assets (grammar asset)"),
    );
    let clients = CoordinatorClients {
        analyzer: Some(analyzer),
        reformulator: None,
        compressor: None,
        verifier: None,
        filter: None,
        router: None,
        slm_circuits: std::collections::HashMap::new(),
    };
    let reg = ProviderRegistry::new();
    reg.register(claude);
    let coord = Coordinator::builder().registry(reg).clients(clients).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert(cid.clone(), meta());
    let req = MissionARequest {
        user_text: "Quelle est la capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![cid.clone()],
        llm_metadata: metas,
        budget: budget(),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(30),
        task_class_scoring_table: None,
        tenant_id: None,
    };
    let resp = tokio::time::timeout(MISSION_DEADLINE, run_mission_a(&coord, req))
        .await
        .expect("pas de hang")
        .expect("mission A survit malgré analyzer mort (mode dégradé)");

    eprintln!("\n=== R3.S6 live chaos SLM analyzer down ===");
    eprintln!("  final: {:?} | failure_mode: {}", resp.final_text, resp.failure_mode.summarize());
    // Résilience : la mission a quand même répondu (texte brut → LLM).
    assert!(!resp.final_text.is_empty(), "réponse produite malgré analyzer mort");
}
