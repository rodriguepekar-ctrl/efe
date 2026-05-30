// SPDX-License-Identifier: UNLICENSED
//! **R3.S7 — Tests d'intégration télémétrie.**
//!
//! - Hook lossless : le `Coordinator` enregistre les métriques (succès + erreur)
//!   après chaque mission. Ces tests tournent **sans** la feature `metrics`
//!   (le `MetricsRegistry` + le hook sont toujours compilés).
//! - Serveur HTTP `/metrics` : sous `#[cfg(feature = "metrics")]`, le serveur
//!   répond du format Prometheus valide + reflète l'état live des circuits.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, run_mission_b, run_speculative, Coordinator, MetricsRegistry, MissionARequest,
    MissionBRequest, MissionType, MockLlmProvider, Outcome, ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 100,
        p99_latency_ms: 500,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

fn budget() -> Budget {
    Budget {
        max_cost_cents: 1000,
        max_cost_microcents: 0,
        max_latency_ms: 10_000,
        min_quality_score: 0.0,
    }
}

fn mission_a_req(llm: &str) -> MissionARequest {
    let mut metas = HashMap::new();
    metas.insert(llm.to_string(), meta());
    MissionARequest {
        user_text: "Capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm.to_string()],
        llm_metadata: metas,
        budget: budget(),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
        tenant_id: None,
    }
}

// =====================================================================
// #8 — Le coordinator enregistre les métriques d'une mission réussie
// =====================================================================

#[tokio::test]
async fn coordinator_records_mission_metrics_after_run() {
    let reg = Arc::new(MetricsRegistry::new());
    let preg = ProviderRegistry::new();
    preg.register(Arc::new(
        MockLlmProvider::new("claude")
            .with_response("Paris")
            .with_cost_microcents(500),
    ));
    let coord = Coordinator::builder()
        .registry(preg)
        .metrics(reg.clone())
        .build()
        .unwrap();

    let resp = run_mission_a(&coord, mission_a_req("claude")).await.unwrap();
    assert_eq!(resp.llm_used, "claude");

    // Hook lossless : 1 mission A réussie, coût attribué au provider.
    assert_eq!(reg.missions_count(MissionType::A, Outcome::Success), 1);
    assert_eq!(reg.missions_count(MissionType::A, Outcome::Error), 0);
    assert_eq!(reg.cost_microcents_of("claude"), 500);
}

// =====================================================================
// #9 — Le coordinator compte les erreurs (guard RAII via Drop)
// =====================================================================

#[tokio::test]
async fn coordinator_records_error_outcome() {
    let reg = Arc::new(MetricsRegistry::new());
    let preg = ProviderRegistry::new();
    // Seul candidat → faute systématique → tous candidats épuisés → Err.
    preg.register(Arc::new(
        MockLlmProvider::new("claude").with_error("HTTP 500: down"),
    ));
    let coord = Coordinator::builder()
        .registry(preg)
        .metrics(reg.clone())
        .build()
        .unwrap();

    let r = run_mission_a(&coord, mission_a_req("claude")).await;
    assert!(r.is_err(), "tous les candidats échouent → Err");

    // Le guard RAII a enregistré outcome=error via Drop (lossless).
    assert_eq!(reg.missions_count(MissionType::A, Outcome::Error), 1);
    assert_eq!(reg.missions_count(MissionType::A, Outcome::Success), 0);
}

// =====================================================================
// Mission B : exerce le chemin d'enregistrement `from_mission_b`
// =====================================================================

#[tokio::test]
async fn coordinator_records_mission_b_metrics() {
    let reg = Arc::new(MetricsRegistry::new());
    let preg = ProviderRegistry::new();
    for id in ["a", "b", "c"] {
        preg.register(Arc::new(MockLlmProvider::new(id).with_response("Paris")));
    }
    let coord = Coordinator::builder()
        .registry(preg)
        .metrics(reg.clone())
        .build()
        .unwrap();

    let mut metas = HashMap::new();
    for id in ["a", "b", "c"] {
        metas.insert(id.to_string(), meta());
    }
    let req = MissionBRequest {
        user_text: "Capitale ?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into(), "c".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        tenant_id: None,
    };
    let resp = run_mission_b(&coord, req).await.unwrap();

    // from_mission_b : 1 mission B réussie, coût agrégé étiqueté par le winner.
    assert_eq!(reg.missions_count(MissionType::B, Outcome::Success), 1);
    assert!(
        reg.cost_microcents_of(&resp.winner_llm) > 0
            || resp.total_cost_microcents == 0,
        "coût agrégé attribué au winner ({})",
        resp.winner_llm
    );
}

// =====================================================================
// Speculative : exerce le chemin d'enregistrement `from_speculative`
// =====================================================================

#[tokio::test]
async fn coordinator_records_speculative_metrics() {
    use efe_coordinator::SavingsMode;
    let reg = Arc::new(MetricsRegistry::new());
    let preg = ProviderRegistry::new();
    preg.register(Arc::new(MockLlmProvider::new("drafter").with_response("Paris")));
    preg.register(Arc::new(MockLlmProvider::new("verifier").with_response("Paris")));
    let coord = Coordinator::builder()
        .registry(preg)
        .metrics(reg.clone())
        .build()
        .unwrap();

    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Capitale de la France ?".into(),
        TaskClass::FactualQa,
        "drafter".into(),
        "verifier".into(),
    );
    req.max_words_for_quick_accept = 10; // hot path (« Paris » ≤ 10 mots)

    let resp = run_speculative(&coord, req).await.unwrap();

    // from_speculative : 1 mission Speculative réussie.
    assert_eq!(reg.missions_count(MissionType::Speculative, Outcome::Success), 1);
    // Hot path → verifier skippé → savings enregistrés (si l'estimation > 0).
    eprintln!(
        "  speculative: verifier_skipped={} savings_µ¢={}",
        resp.verifier_skipped,
        reg.savings_microcents_of(SavingsMode::Speculative)
    );
    if resp.verifier_skipped {
        // Cohérence : pas de double-comptage erreur, savings ≥ 0 (enregistrés
        // via le chemin from_speculative quand spec_savings_microcents > 0).
        assert_eq!(reg.missions_count(MissionType::Speculative, Outcome::Error), 0);
    }
}

// =====================================================================
// #6bis — metrics None ⇒ aucune régression (overhead nul)
// =====================================================================

#[tokio::test]
async fn coordinator_without_metrics_runs_normally() {
    let preg = ProviderRegistry::new();
    preg.register(Arc::new(MockLlmProvider::new("claude").with_response("Paris")));
    // Pas de .metrics(...) → coord.metrics == None → guard inerte.
    let coord = Coordinator::builder().registry(preg).build().unwrap();
    let resp = run_mission_a(&coord, mission_a_req("claude")).await.unwrap();
    assert_eq!(resp.final_text, "Paris");
}

// =====================================================================
// Serveur HTTP /metrics (feature `metrics`)
// =====================================================================

#[cfg(feature = "metrics")]
mod server_tests {
    use super::*;
    use efe_coordinator::metrics_server::serve_metrics_on;
    use efe_coordinator::{MissionRecord, Phase, Verdict};
    use efe_nano_app_coordinator::SlmCircuitBreaker;
    use tokio::net::TcpListener;

    async fn spawn_server(
        reg: Arc<MetricsRegistry>,
        clients: Arc<efe_coordinator::CoordinatorClients>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_metrics_on(listener, reg, clients));
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn metrics_server_get_returns_200_prometheus() {
        let reg = Arc::new(MetricsRegistry::new());
        // Pré-remplit une métrique pour avoir un sample non trivial.
        reg.record_mission(&MissionRecord {
            ty: MissionType::A,
            outcome: Outcome::Success,
            provider: "claude".into(),
            cost_microcents: 42,
            phase_latencies: vec![(Phase::Llm, 120)],
            savings: None,
            verdicts: vec![Verdict::Supported],
            failure_kinds: vec![],
        });
        let coord = Coordinator::builder().build().unwrap();
        let base = spawn_server(reg, coord.clients.clone()).await;

        let resp = reqwest::get(format!("{base}/metrics")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(ct.starts_with("text/plain"), "content-type = {ct}");
        let body = resp.text().await.unwrap();
        assert!(body.contains("efe_missions_total{type=\"A\",outcome=\"success\"} 1"));
        assert!(body.contains("efe_cost_microcents_total{provider=\"claude\"} 42"));
        assert!(body.contains("# TYPE efe_latency_ms histogram"));
        // Anti-cardinalité : aucun label tenant.
        assert!(!body.to_lowercase().contains("tenant"));
    }

    #[tokio::test]
    async fn metrics_server_unknown_route_404() {
        let reg = Arc::new(MetricsRegistry::new());
        let coord = Coordinator::builder().build().unwrap();
        let base = spawn_server(reg, coord.clients.clone()).await;
        let resp = reqwest::get(format!("{base}/nope")).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn metrics_server_renders_live_circuit_state() {
        let reg = Arc::new(MetricsRegistry::new());
        // Circuit "analyzer" pré-ouvert.
        let cb = SlmCircuitBreaker::new(1, Duration::from_secs(3600));
        cb.record_failure(); // → Open
        let mut circuits = HashMap::new();
        circuits.insert("analyzer".to_string(), cb);
        let coord = Coordinator::builder()
            .slm_circuits(circuits)
            .build()
            .unwrap();
        let base = spawn_server(reg, coord.clients.clone()).await;

        let body = reqwest::get(format!("{base}/metrics"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // Gauge scrape-time : circuit ouvert → open=1, closed=0.
        assert!(body.contains("efe_slm_circuit_state{robot=\"analyzer\",state=\"open\"} 1"));
        assert!(body.contains("efe_slm_circuit_state{robot=\"analyzer\",state=\"closed\"} 0"));
    }

    #[tokio::test]
    async fn metrics_server_healthz_ok() {
        let reg = Arc::new(MetricsRegistry::new());
        let coord = Coordinator::builder().build().unwrap();
        let base = spawn_server(reg, coord.clients.clone()).await;
        let resp = reqwest::get(format!("{base}/healthz")).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
