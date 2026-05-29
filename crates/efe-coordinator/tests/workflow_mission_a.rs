// SPDX-License-Identifier: UNLICENSED
//! Workflow Mission A end-to-end (sans nano-robots — provider mock).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, Coordinator, MissionARequest, MockLlmProvider, ProviderRegistry,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

#[tokio::test]
async fn mission_a_runs_with_mock_llm_only() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response("the answer is X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "Tell me X".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();
    assert_eq!(resp.llm_used, "claude_mock");
    assert_eq!(resp.final_text, "the answer is X");
    assert!(resp.audit_chain_hash.len() == 64);
    assert!(resp.latency_ms < 5000);
}

#[tokio::test]
async fn mission_a_publishes_swarm_started_and_finished() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("done")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("test-ui"));

    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let _resp = run_mission_a(&coord, req).await.unwrap();

    // Drain le canal en non-bloquant.
    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }
    assert!(tags.contains(&"SWARM_STARTED"), "tags={:?}", tags);
    assert!(tags.contains(&"SWARM_FINISHED"), "tags={:?}", tags);
    assert!(tags.contains(&"LLM_FINISHED"), "tags={:?}", tags);
}

#[tokio::test]
async fn mission_a_with_no_available_llm_errors() {
    let coord = Coordinator::for_tests();
    let req = MissionARequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![],
        llm_metadata: HashMap::new(),
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 1000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let r = run_mission_a(&coord, req).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn mission_a_falls_back_to_first_llm_if_router_absent() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("first").with_response("FIRST")));
    reg.register(Arc::new(MockLlmProvider::new("second").with_response("SECOND")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = MissionARequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["first".into(), "second".into()],
        llm_metadata: HashMap::new(),
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 1000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();
    assert_eq!(resp.llm_used, "first");
}

// ─── Sprint S4 R2 : couverture offline de la nouvelle instrumentation ───────
//
// Ces tests valident en offline (avec MockLlmProvider, sans nano-robots) que
// `run_mission_a` remplit correctement le champ `MissionAResponse.chewing_metrics`
// et émet l'event `StreamEvent::PreChewingMetrics`. Sans ces tests, les
// modifications source de S4 ne sont validées qu'en mode live (qui requiert
// EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1 + 3 daemons SLM UP).

#[tokio::test]
async fn mission_a_chewing_metrics_filled_with_mocks() {
    // ── Setup minimal : 1 mock LLM, 0 nano-robot ────────────────────────
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response("answer")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());

    // user_text court (< 500 chars) → compressor skip → ratio = 1.0
    let user_text = "Tell me X";
    let req = MissionARequest {
        tenant_id: None,
        user_text: user_text.into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();

    // ── Le champ chewing_metrics doit être Some(...) systématiquement ───
    let m = resp
        .chewing_metrics
        .as_ref()
        .expect("S4: MissionAResponse.chewing_metrics doit être Some(...) après run_mission_a");

    // ── input_tokens_raw : ceil(9/4) = 3 (pour "Tell me X") ─────────────
    assert_eq!(
        m.input_tokens_raw,
        3,
        "estimate_tokens('Tell me X') = ceil(9/4) = 3, got {}",
        m.input_tokens_raw
    );

    // ── Sans compressor : compressed == raw, ratio = 1.0, savings = 0 ───
    assert_eq!(
        m.input_tokens_compressed, m.input_tokens_raw,
        "Sans compressor (clients.compressor = None), compressed == raw"
    );
    assert_eq!(m.compression_ratio, 1.0, "ratio = 1.0 quand pas de compression");
    assert_eq!(m.savings_tokens, 0);
    assert_eq!(m.savings_ratio, 0.0);

    // ── selected_llm = "claude_mock" (forcé via available_llms.first()) ─
    assert_eq!(m.selected_llm, "claude_mock");

    // ── Sans router : pas de reason, latency = 0 ────────────────────────
    assert!(
        m.router_reason.is_empty(),
        "Sans router nano-robot, router_reason reste vide"
    );
    assert_eq!(
        m.router_latency_ms, 0,
        "Sans router, router_latency_ms reste 0"
    );

    // ── Sans analyzer / compressor / verifier : leurs latences = 0 ──────
    assert_eq!(m.analyzer_latency_ms, 0);
    assert_eq!(m.compressor_latency_ms, 0);
    assert_eq!(m.verifier_latency_ms, 0);

    // ── total_slm_latency_ms = 0 (somme des 3 ci-dessus) ────────────────
    assert_eq!(m.total_slm_latency_ms, 0);

    // ── llm_latency_ms : non zéro (le mock retourne un peu de latency) ──
    // MockLlmProvider retourne par défaut latency_ms = 1.
    assert!(
        m.llm_latency_ms <= 1000,
        "llm_latency_ms ≤ 1000 ms (mock rapide), got {}",
        m.llm_latency_ms
    );

    // ── total_pipeline_latency_ms > 0 (le mission_a a tourné) ───────────
    assert!(
        m.total_pipeline_latency_ms > 0,
        "total_pipeline_latency_ms > 0 après une mission complète"
    );

    // ── Cohérence : pipeline >= slm (qui est 0 ici) ─────────────────────
    assert!(m.total_pipeline_latency_ms >= m.total_slm_latency_ms);

    // ── verdicts vide (pas de verifier) ─────────────────────────────────
    assert!(resp.verdicts.is_empty());
}

#[tokio::test]
async fn mission_a_emits_pre_chewing_metrics_event() {
    // Vérifie que l'event PRE_CHEWING_METRICS est bien émis sur le bus
    // par run_mission_a, en mode offline (mock LLM).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("x")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("s4-offline-event-test"));

    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let _resp = run_mission_a(&coord, req).await.unwrap();

    // Drain events.
    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }

    // L'event PRE_CHEWING_METRICS doit être présent.
    assert!(
        tags.contains(&"PRE_CHEWING_METRICS"),
        "S4: PRE_CHEWING_METRICS doit être émis par run_mission_a, got tags={tags:?}"
    );

    // Et il doit arriver AVANT SWARM_FINISHED (ordre du workflow).
    let pre_chew_pos = tags.iter().position(|t| *t == "PRE_CHEWING_METRICS").unwrap();
    let finished_pos = tags
        .iter()
        .position(|t| *t == "SWARM_FINISHED")
        .expect("SWARM_FINISHED doit être présent");
    assert!(
        pre_chew_pos < finished_pos,
        "S4: PRE_CHEWING_METRICS (pos {pre_chew_pos}) doit précéder SWARM_FINISHED (pos {finished_pos})"
    );
}

#[tokio::test]
async fn mission_a_chewing_metrics_serializable_through_response() {
    // Vérifie que MissionAResponse.chewing_metrics fait correctement le
    // roundtrip JSON via serde (utilisé pour exposer la response via API).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("x")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();

    // Sérialise → désérialise → compare.
    let json = serde_json::to_string(&resp).unwrap();
    assert!(
        json.contains("\"chewing_metrics\""),
        "Le JSON sérialisé doit contenir le champ chewing_metrics"
    );
    let back: efe_coordinator::MissionAResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.final_text, resp.final_text);
    assert!(back.chewing_metrics.is_some());
}

#[tokio::test]
async fn mission_a_backward_compat_old_response_without_chewing_metrics() {
    // Garantit que la backward compat #[serde(default)] fonctionne :
    // un MissionAResponse sérialisé SANS le champ chewing_metrics
    // (par exemple par un ancien serveur ou un test offline antérieur)
    // doit toujours désérialiser correctement avec chewing_metrics = None.
    let old_json = r#"{
        "final_text": "Paris est la capitale",
        "llm_used": "claude_old",
        "cost_cents": 1,
        "latency_ms": 1500,
        "verdicts": [],
        "audit_chain_hash": "abc123"
    }"#;
    let parsed: efe_coordinator::MissionAResponse =
        serde_json::from_str(old_json).expect("backward compat #[serde(default)] doit fonctionner");
    assert!(
        parsed.chewing_metrics.is_none(),
        "chewing_metrics doit être None pour un JSON pré-S4 (#[serde(default)])"
    );
    assert_eq!(parsed.final_text, "Paris est la capitale");
}

// ─── Sprint S6 R2 — Tests offline d'intégration du router tuning ──────────
//
// Ces 3 tests prouvent en OFFLINE (avec MockProviders, sans clés API) que le
// wiring `task_class_scoring_table → RouterClient::for_task_class → invoke`
// route bien vers le LLM attendu DANS le workflow `run_mission_a` complet
// (pas juste dans les tests isolés du module `scoring` qui n'instanciaient
// que le router sans pipeline complet).
//
// Pourquoi c'est crucial : les tests du module `scoring` valident la table
// + le scoring Pareto, mais PAS l'intégration dans `run_mission_a` (capture
// du `selected_llm`, propagation vers `resp.llm_used`, etc.). Sans ces
// tests, un bug d'integration (ex: oubli de propager `selected`) serait
// invisible jusqu'au test live à 3-5 ¢.

/// Helper S6 offline : enregistre les 3 providers réels (claude_…, gemini_…,
/// gpt_…) mais avec MockLlmProvider pour ne pas appeler les vrais LLMs.
///
/// Les IDs matchent EXACTEMENT ceux de la table `default_3llm_table()`
/// pour que le routing puisse résoudre les capability_scores.
fn make_coord_with_3_mock_providers() -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude_claude-sonnet-4-6")
            .with_response("Claude offline response"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_gemini-2.5-flash")
            .with_response("Gemini offline response"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gpt_gpt-4o").with_response("GPT offline response"),
    ));
    Coordinator::builder().registry(reg).build().unwrap()
}

/// **S6 offline intégration** : avec la table par défaut + 3 mock providers
/// + `TaskClass::Code`, `run_mission_a` doit choisir Claude (winner Code
/// quality-driven, weights q=2.5, c=0.2).
///
/// Prouve que :
/// 1. Le champ `task_class_scoring_table` est bien consommé par `run_mission_a`
/// 2. `RouterClient::for_task_class` est bien construit avec les bons weights
/// 3. Le router retourne `selected = "claude_…"` qui propage à `resp.llm_used`
/// 4. La réponse vient bien du provider Claude (pas Gemini ou GPT)
#[tokio::test]
async fn mission_a_with_scoring_table_picks_claude_for_code_offline() {
    let coord = make_coord_with_3_mock_providers();
    let req = MissionARequest::with_default_scoring(
        "Écris une fonction Rust fibonacci".to_string(),
        TaskClass::Code,
        vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ],
    );
    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a doit succeed avec 3 mock providers + table de scoring");

    // Winner Code = Claude (cf. scoring::tests::code_picks_claude)
    assert_eq!(
        resp.llm_used, "claude_claude-sonnet-4-6",
        "Code task_class doit router vers Claude (HumanEval cap=0.95 vs 0.72/0.85)"
    );
    // La réponse doit venir du mock Claude.
    assert_eq!(resp.final_text, "Claude offline response");
    // router_reason rempli = preuve que le router a tourné.
    let metrics = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert!(
        !metrics.router_reason.is_empty(),
        "router_reason doit être rempli (preuve que le router tuné a invoqué)"
    );
    assert!(
        metrics.router_reason.contains("claude"),
        "router_reason doit mentionner le LLM choisi, got: {}",
        metrics.router_reason
    );
}

/// **S6 offline intégration (variante)** : avec `TaskClass::Translation`, le
/// même set de providers doit router vers **Gemini** (cost-driven, weights
/// q=1.0, c=2.0). Démontre que la même table donne des décisions
/// DIFFÉRENTES selon la task_class.
#[tokio::test]
async fn mission_a_with_scoring_table_picks_gemini_for_translation_offline() {
    let coord = make_coord_with_3_mock_providers();
    let req = MissionARequest::with_default_scoring(
        "Traduis : Le crépuscule baigne Paris".to_string(),
        TaskClass::Translation,
        vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ],
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    // Winner Translation = Gemini (cost 30x moins cher, cap 0.92)
    assert_eq!(
        resp.llm_used, "gemini_gemini-2.5-flash",
        "Translation task_class doit router vers Gemini (cost-driven c=2.0)"
    );
    assert_eq!(resp.final_text, "Gemini offline response");
}

/// **S6 offline intégration (3e variante)** : avec `TaskClass::Creative`, le
/// router doit choisir **GPT-4o** (capability 0.93 — meilleur sur style/variété).
#[tokio::test]
async fn mission_a_with_scoring_table_picks_gpt_for_creative_offline() {
    let coord = make_coord_with_3_mock_providers();
    let req = MissionARequest::with_default_scoring(
        "Écris un haïku sur les huitres".to_string(),
        TaskClass::Creative,
        vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ],
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    assert_eq!(
        resp.llm_used, "gpt_gpt-4o",
        "Creative task_class doit router vers GPT-4o (cap=0.93 vs 0.88/0.80)"
    );
    assert_eq!(resp.final_text, "GPT offline response");
}

/// **S6 offline backward-compat** : sans `task_class_scoring_table` et sans
/// `coord.clients.router`, le workflow doit tomber sur le 1er LLM disponible.
/// Comportement S2-S5 préservé.
#[tokio::test]
async fn mission_a_without_scoring_table_falls_back_pre_s6_behavior() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude_first").with_response("first response"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_second").with_response("second response"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_first".to_string(), meta());
    metas.insert("gemini_second".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::Code, // task_class IGNORÉ sans table
        available_llms: vec!["claude_first".into(), "gemini_second".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        // Pas de table → fallback first LLM (pas de routing).
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();
    // Sans router + sans table → prend le 1er. Code n'est PAS routé vers
    // Claude car la décision dépendait de la table.
    assert_eq!(resp.llm_used, "claude_first");
    assert_eq!(resp.final_text, "first response");
}

// ====================================================================
// R3.S1 — Tests intégration workflow microcents (Gap H)
// ====================================================================

/// **R3.S1 (§3.11)** : assertion µ¢ end-to-end sur Mission A.  Vérifie que
/// `cost_microcents` est non-zéro et que `cost_cents` est cohérent avec
/// (ou ≤ que) `microcents_to_cents(cost_microcents)`.
#[tokio::test]
async fn r3s1_mission_a_propagates_cost_microcents_end_to_end() {
    let reg = ProviderRegistry::new();
    // MockLlmProvider explicitement à 45 µ¢ (= Claude fixture 01 réel).
    reg.register(Arc::new(
        MockLlmProvider::new("claude_mock")
            .with_response("Paris")
            .with_cost_microcents(45),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());

    let req = MissionARequest {
        tenant_id: None,
        user_text: "What is the capital of France?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    };
    let resp = run_mission_a(&coord, req).await.unwrap();

    // R3.S1 : cost_microcents propagé EXACT depuis le mock.
    assert_eq!(
        resp.cost_microcents, 45,
        "MissionAResponse.cost_microcents doit refléter le mock provider (45 µ¢)"
    );
    // cost_cents dérivé via policy ≥ 1 si > 0 µ¢ (45 µ¢ → 1 cent).
    assert_eq!(
        resp.cost_cents, 1,
        "cost_cents dérivé via microcents_to_cents (45 µ¢ < 500 → policy = 1)"
    );
}

/// **R3.S1 (§3.11)** : assertion µ¢ — vérifie que `chewing_metrics.
/// router_estimated_cost_microcents` est peuplé quand le router est utilisé.
#[tokio::test]
async fn r3s1_mission_a_chewing_metrics_router_estimated_cost_microcents() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude_claude-sonnet-4-6")
            .with_response("ok")
            .with_cost_microcents(150),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_gemini-2.5-flash")
            .with_response("ok")
            .with_cost_microcents(5),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("gpt_gpt-4o").with_response("ok").with_cost_microcents(100),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    // Use the default scoring table → router activé.
    let req = MissionARequest::with_default_scoring(
        "Translate: Bonjour".to_string(),
        TaskClass::Translation, // cost-driven → Gemini
        vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ],
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    // Translation cost-driven → Gemini.
    assert_eq!(resp.llm_used, "gemini_gemini-2.5-flash");

    // chewing_metrics présent.
    let cm = resp
        .chewing_metrics
        .as_ref()
        .expect("chewing_metrics doit exister");
    // R3.S1 : router_estimated_cost_microcents doit être > 0.
    assert!(
        cm.router_estimated_cost_microcents > 0,
        "router_estimated_cost_microcents doit être peuplé (got {})",
        cm.router_estimated_cost_microcents
    );
    // Cost cents legacy aussi peuplé.
    assert!(cm.router_estimated_cost_cents <= cm.router_estimated_cost_microcents as u32 * 1000);
}

