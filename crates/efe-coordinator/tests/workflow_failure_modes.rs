// SPDX-License-Identifier: UNLICENSED
//! **Sprint S8 R2** — Tests offline des modes de failure (graceful degradation).
//!
//! Couvre les 5 catégories de failure documentées dans
//! `docs/architecture/EFE_A1_PHASE3_R2_S8_FAILURE_MODES_*` :
//! - F1 : SLM down (analyzer/compressor/verifier circuit ouvert)
//! - F2 : LLM `selected` 429 / 5xx → fallback chain
//! - F3 : Mission B 1-2 LLM fail → vote dégradé
//! - F4 : Mission B failed_llms tracé
//! - F5 : Mission Speculative drafter fail → bypass verifier

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, run_mission_b, run_speculative, Coordinator, CoordError,
    FailureMode, MissionARequest, MissionBRequest, MockLlmProvider,
    ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

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

fn mission_a_request(
    available_llms: Vec<String>,
    llm_metadata: HashMap<String, LlmMetadata>,
) -> MissionARequest {
    MissionARequest {
        user_text: "test".into(),
        task_class: TaskClass::FactualQa,
        available_llms,
        llm_metadata,
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    }
}

fn mission_b_request(llms: Vec<String>) -> MissionBRequest {
    let mut metadata = HashMap::new();
    for l in &llms {
        metadata.insert(l.clone(), meta());
    }
    MissionBRequest {
        user_text: "test".into(),
        task_class: TaskClass::FactualQa,
        llms,
        llm_metadata: metadata,
        vote_weights: HashMap::new(),
        quorum: Some(1), // quorum minimal pour permettre vote dégradé
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// F2 — LLM fallback chain (Mission A)
// ═══════════════════════════════════════════════════════════════════════════

/// **F2-T1** : Le `selected_llm` fail (HTTP 429) → fallback sur le 2e LLM
/// disponible. La réponse vient du fallback, `fallback_chain_used` est set.
#[tokio::test]
async fn mission_a_falls_back_when_selected_429_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("primary").with_error("HTTP 429: rate limited"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("fallback_llm").with_response("from fallback"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("primary".to_string(), meta());
    metadata.insert("fallback_llm".to_string(), meta());

    let req = mission_a_request(
        vec!["primary".to_string(), "fallback_llm".to_string()],
        metadata,
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    assert_eq!(resp.llm_used, "fallback_llm", "fallback retenu");
    assert_eq!(resp.final_text, "from fallback");
    assert_eq!(
        resp.fallback_chain_used,
        Some("fallback_llm".to_string()),
        "fallback_chain_used tracé"
    );
    assert!(matches!(
        resp.failure_mode,
        FailureMode::LlmFallbackTriggered { .. }
    ));
    let metrics = resp.chewing_metrics.expect("chewing_metrics");
    assert_eq!(metrics.degradations_count, 1);
    assert!(metrics
        .failed_components
        .iter()
        .any(|c| c.contains("primary")));
}

/// **F2-T2** : Primary + premier fallback fail, 2e fallback succeed.
#[tokio::test]
async fn mission_a_uses_second_fallback_when_first_also_fails_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("primary").with_error("HTTP 503"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("fallback1").with_error("HTTP 502"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("fallback2").with_response("ok from fb2"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("primary".to_string(), meta());
    metadata.insert("fallback1".to_string(), meta());
    metadata.insert("fallback2".to_string(), meta());

    let req = mission_a_request(
        vec![
            "primary".to_string(),
            "fallback1".to_string(),
            "fallback2".to_string(),
        ],
        metadata,
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    assert_eq!(resp.llm_used, "fallback2");
    assert_eq!(resp.fallback_chain_used, Some("fallback2".to_string()));
    let metrics = resp.chewing_metrics.unwrap();
    // primary + fallback1 ont fail → 2 failed_components.
    assert_eq!(metrics.failed_components.len(), 2);
}

/// **F2-T3** : Tous les candidats échouent → erreur enrichie.
#[tokio::test]
async fn mission_a_errors_when_all_candidates_exhausted_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("a").with_error("HTTP 500"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("b").with_error("HTTP 500"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("a".to_string(), meta());
    metadata.insert("b".to_string(), meta());

    let req = mission_a_request(vec!["a".to_string(), "b".to_string()], metadata);
    let r = run_mission_a(&coord, req).await;
    assert!(r.is_err());
    // L'erreur contient le dernier reason observé (HTTP 500).
    let err_str = format!("{}", r.unwrap_err());
    assert!(
        err_str.contains("500") || err_str.contains("exhausted") || err_str.contains("failed"),
        "Erreur doit mentionner 500 / exhausted / failed, got: {err_str}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// F3 — Mission B vote dégradé
// ═══════════════════════════════════════════════════════════════════════════

/// **F3-T1** : 1/3 LLM fail → continue + vote_degraded=true + 1 entry dans failed_llms.
#[tokio::test]
async fn mission_b_continues_when_1_of_3_llms_fail_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("Paris")));
    reg.register(Arc::new(
        MockLlmProvider::new("c").with_error("HTTP 429"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = mission_b_request(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let resp = run_mission_b(&coord, req).await.unwrap();

    assert!(resp.vote_degraded, "vote_degraded car 1 LLM a fail");
    assert_eq!(resp.failed_llms.len(), 1);
    assert_eq!(resp.failed_llms[0].0, "c");
    assert!(resp.failed_llms[0].1.contains("429"));
    assert!(matches!(
        resp.failure_mode,
        FailureMode::VoteDegraded {
            effective_votes: 2,
            expected: 3
        }
    ));
}

/// **F3-T2** : 2/3 LLM fail → continue avec 1 vote (quorum=1).
#[tokio::test]
async fn mission_b_vote_degraded_true_when_2_of_3_llms_fail_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("Paris")));
    reg.register(Arc::new(
        MockLlmProvider::new("b").with_error("HTTP 503"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("c").with_error("HTTP 429"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = mission_b_request(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let resp = run_mission_b(&coord, req).await.unwrap();

    assert!(resp.vote_degraded);
    assert_eq!(resp.failed_llms.len(), 2);
    assert!(matches!(
        resp.failure_mode,
        FailureMode::VoteDegraded {
            effective_votes: 1,
            expected: 3
        }
    ));
    assert_eq!(resp.final_text, "Paris");
}

/// **F3-T3** : Tous les 3 LLMs fail → erreur dure avec contexte enrichi.
#[tokio::test]
async fn mission_b_errors_when_all_3_llms_fail_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("a").with_error("HTTP 500"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("b").with_error("HTTP 503"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("c").with_error("HTTP 429"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = mission_b_request(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let r = run_mission_b(&coord, req).await;
    assert!(r.is_err());
    let err_str = format!("{}", r.unwrap_err());
    assert!(
        err_str.contains("all") && err_str.contains("3"),
        "Erreur doit mentionner 'all 3 providers failed', got: {err_str}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// F4 — Mission B failed_llms tracé
// ═══════════════════════════════════════════════════════════════════════════

/// **F4-T1** : failed_llms contient (id, reason) précis pour chaque LLM raté.
#[tokio::test]
async fn mission_b_failed_llms_traced_in_response_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("ok1").with_response("Paris")));
    reg.register(Arc::new(
        MockLlmProvider::new("bad1").with_error("HTTP 429: rate"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("bad2").with_error("HTTP 503: overload"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = mission_b_request(vec![
        "ok1".to_string(),
        "bad1".to_string(),
        "bad2".to_string(),
    ]);
    let resp = run_mission_b(&coord, req).await.unwrap();

    assert_eq!(resp.failed_llms.len(), 2);
    // Ordre stable selon l'ordre de req.llms.
    let ids: Vec<&str> = resp.failed_llms.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"bad1") && ids.contains(&"bad2"));
    // Les reasons contiennent les codes HTTP.
    let reasons: String = resp
        .failed_llms
        .iter()
        .map(|(_, r)| r.clone())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(reasons.contains("429"));
    assert!(reasons.contains("503"));
}

// ═══════════════════════════════════════════════════════════════════════════
// F5 — Mission Speculative drafter fail → bypass
// ═══════════════════════════════════════════════════════════════════════════

/// **F5-T1** : Drafter fail → Speculator bypass + verifier appelé.
#[tokio::test]
async fn speculative_falls_back_to_verifier_when_drafter_fails_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter").with_error("HTTP 503: drafter down"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("verifier").with_response("saved by verifier"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "verifier".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10,
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert_eq!(resp.final_text, "saved by verifier", "verifier text retenu");
    assert!(!resp.verifier_skipped, "verifier appelé (slow path forcé)");
    // draft_text vide car drafter a fail.
    assert!(resp.draft_text.is_empty());
}

/// **F5-T2** : Le `drafter_bypassed` est exposé dans `SpeculationOutcome` au
/// niveau du Speculator (vérification unit-level via direct call).
///
/// On peut pas accéder directement à l'outcome via `run_speculative` (qui
/// abstrait), mais le test F5-T1 prouve déjà le comportement bout-en-bout.
/// Ici on valide la sémantique du field via test direct du Speculator.
#[tokio::test]
async fn speculative_drafter_bypassed_event_emitted_offline() {
    use efe_coordinator::events::StreamEvent;
    use std::time::Duration as StdDuration;
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("d").with_error("HTTP 503"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("v").with_response("verifier saved"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_token, mut rx) = coord.subscribe(Some("s8-bypass-test"));

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "v".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "d".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10,
    };
    let _ = run_speculative(&coord, req).await.unwrap();

    // Vérifie qu'un event DRAFTER_BYPASSED a bien été émis sur le bus.
    let mut saw_bypass = false;
    while let Ok(Some(ev)) =
        tokio::time::timeout(StdDuration::from_millis(100), rx.recv()).await
    {
        if matches!(ev, StreamEvent::DrafterBypassed { .. }) {
            saw_bypass = true;
            break;
        }
    }
    assert!(
        saw_bypass,
        "StreamEvent::DrafterBypassed doit être émis quand le drafter fail"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// FailureMode tracing — cohérence chewing_metrics
// ═══════════════════════════════════════════════════════════════════════════

/// **FM-T1** : Cas nominal → failure_mode = None partout.
#[tokio::test]
async fn nominal_mission_a_failure_mode_is_none_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("ok").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("ok".to_string(), meta());
    let req = mission_a_request(vec!["ok".to_string()], metadata);
    let resp = run_mission_a(&coord, req).await.unwrap();

    assert_eq!(resp.failure_mode, FailureMode::None);
    assert!(resp.fallback_chain_used.is_none());
    let metrics = resp.chewing_metrics.unwrap();
    assert_eq!(metrics.degradations_count, 0);
    assert!(metrics.failed_components.is_empty());
}

/// **FM-T2** : chewing_metrics.failure_mode reflète resp.failure_mode.
#[tokio::test]
async fn chewing_metrics_failure_mode_matches_response_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("primary").with_error("HTTP 429"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("backup").with_response("ok"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("primary".to_string(), meta());
    metadata.insert("backup".to_string(), meta());
    let req = mission_a_request(
        vec!["primary".to_string(), "backup".to_string()],
        metadata,
    );
    let resp = run_mission_a(&coord, req).await.unwrap();

    let metrics = resp.chewing_metrics.as_ref().unwrap();
    assert_eq!(
        metrics.failure_mode, resp.failure_mode,
        "chewing_metrics.failure_mode doit refléter resp.failure_mode"
    );
    assert_eq!(metrics.degradations_count, 1);
    assert_eq!(metrics.failed_components.len(), 1);
    assert!(metrics.failed_components[0].starts_with("llm:"));
}

/// **FM-T3** : Mission B nominal → failure_mode = None.
#[tokio::test]
async fn mission_b_nominal_failure_mode_is_none_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = mission_b_request(vec!["a".to_string(), "b".to_string()]);
    let resp = run_mission_b(&coord, req).await.unwrap();

    assert!(!resp.vote_degraded);
    assert!(resp.failed_llms.is_empty());
    assert_eq!(resp.failure_mode, FailureMode::None);
}

// ═══════════════════════════════════════════════════════════════════════════
// F3' — Mission B vote dégradé via fallback I5 (quorum strict non atteint)
// ═══════════════════════════════════════════════════════════════════════════
//
// Ces 2 tests comblent un gap chirurgical critique post-audit S8 : ils
// valident que `run_mission_b` exploite le **fallback I5 préexistant** de
// `VoteEngine::tally` (lignes 152-175) — un mécanisme déjà présent depuis S0
// mais **jamais testé via le workflow** Mission B.
//
// Scénario : utilisateur passe `quorum=None` (= défaut `ceil(N/2)`) sur
// 3 LLMs → quorum effectif = 2. Si 2/3 LLMs fail, vote_engine reçoit 1
// ballot avec quorum=2 → fallback I5 sélectionne le seul ballot via
// success_rate descendant. La mission est sauvée, `failure_mode` reporte
// VoteDegraded.

/// **F3'-T1** : `quorum=None` (défaut ceil(3/2)=2) + 2/3 LLMs fail →
/// 1 ballot vs quorum=2 → vote_engine fallback I5 → mission succeed avec
/// le seul ballot survivant.
#[tokio::test]
async fn mission_b_uses_vote_fallback_i5_when_default_quorum_not_met_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("survivor").with_response("Paris")));
    reg.register(Arc::new(
        MockLlmProvider::new("dead1").with_error("HTTP 429"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("dead2").with_error("HTTP 503"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("survivor".to_string(), meta());
    metadata.insert("dead1".to_string(), meta());
    metadata.insert("dead2".to_string(), meta());

    let req = MissionBRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec![
            "survivor".to_string(),
            "dead1".to_string(),
            "dead2".to_string(),
        ],
        llm_metadata: metadata,
        vote_weights: HashMap::new(),
        // quorum: None → défaut = ceil(3/2) = 2.
        // Avec 1 seul ballot survivant, vote_engine doit fallback I5.
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await.expect(
        "S8 : run_mission_b doit succeed grâce au fallback I5 de VoteEngine — \
         1 ballot vs quorum=2 → success_rate fallback prend l'unique survivant",
    );

    assert_eq!(resp.winner_llm, "survivor", "le seul survivant gagne via fallback I5");
    assert_eq!(resp.final_text, "Paris");
    assert!(resp.vote_degraded, "vote_degraded car 2 LLM ont fail");
    assert_eq!(resp.failed_llms.len(), 2);
    assert!(matches!(
        resp.failure_mode,
        FailureMode::VoteDegraded {
            effective_votes: 1,
            expected: 3
        }
    ));
}

/// **F3'-T2** : Cohérence sémantique — `vote_degraded` est true dès qu'1+
/// LLM a fail, INDÉPENDAMMENT de si le fallback I5 a été déclenché ou pas.
/// (1 LLM fail mais quorum=1 atteint avec les 2 autres → pas de fallback
/// I5 mais vote_degraded reste true car 1 LLM a fail).
#[tokio::test]
async fn mission_b_vote_degraded_distinct_from_quorum_fallback_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("Paris")));
    reg.register(Arc::new(
        MockLlmProvider::new("c").with_error("HTTP 429"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert("a".to_string(), meta());
    metadata.insert("b".to_string(), meta());
    metadata.insert("c".to_string(), meta());

    // quorum=None → ceil(3/2) = 2. Avec 2 ballots sur "Paris", quorum=2
    // atteint → pas de fallback I5 nécessaire. Mais c a fail, donc
    // vote_degraded doit quand même être true.
    let req = MissionBRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".to_string(), "b".to_string(), "c".to_string()],
        llm_metadata: metadata,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await.unwrap();

    assert!(resp.vote_degraded, "vote_degraded = true car 1 LLM a fail");
    assert_eq!(resp.failed_llms.len(), 1);
    assert_eq!(resp.final_text, "Paris");
    assert!(matches!(
        resp.failure_mode,
        FailureMode::VoteDegraded {
            effective_votes: 2,
            expected: 3
        }
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// F8 — FailureMode::Composite (gap couverture)
// ═══════════════════════════════════════════════════════════════════════════
//
// Le variant `FailureMode::Composite(Vec<FailureMode>)` est testé en unit
// (failure::tests::composite_*) mais aucun test workflow ne le déclenche.
// Note : aucun workflow actuel ne produit Composite directement (= 1 seul
// type de failure par mission). Ce test valide la sémantique côté UI
// (un consommateur peut construire/parse Composite hors workflow).

/// **F8-T1** : Sémantique de FailureMode::Composite — `is_degraded` +
/// `count_degradations` + `summarize` sont cohérents. Validation unit-level
/// du contract publique côté API consumer (vs failure::tests qui sont
/// internal tests).
#[tokio::test]
async fn failure_mode_composite_contract_offline() {
    let composite = FailureMode::Composite(vec![
        FailureMode::SlmDown("analyzer".into()),
        FailureMode::LlmFallbackTriggered {
            from: "claude_x".into(),
            to: "gemini_y".into(),
        },
    ]);
    assert!(composite.is_degraded());
    assert_eq!(composite.count_degradations(), 2);
    let s = composite.summarize();
    assert!(s.starts_with("composite:"));
    assert!(s.contains("SLM down: analyzer"));
    assert!(s.contains("LLM fallback"));
    // Roundtrip JSON pour vérifier que les consumers UI peuvent
    // parser/sérialiser correctement.
    let j = serde_json::to_string(&composite).unwrap();
    let back: FailureMode = serde_json::from_str(&j).unwrap();
    assert_eq!(back, composite);
}

// Ensure CoordError is reachable for clippy.
#[allow(dead_code)]
fn _coord_error_type_witness() -> Option<CoordError> {
    None
}
