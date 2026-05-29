// SPDX-License-Identifier: UNLICENSED
//! **R3.S5 — Tests d'intégration multi-tenant policies (fix L5).**
//!
//! Valident le wiring des politiques par tenant dans les 3 workflows :
//!
//! - **Backward-compat** : sans registry / sans tenant ⇒ comportement R3.S4.
//! - **Divergence** : 2 tenants → 2 routings + 2 coûts différents (critère
//!   d'acceptation central) via allow-list ET via plafond de coût.
//! - **Allow-list** : intersection (jamais union), fail-closed sur deny-all /
//!   intersection vide.
//! - **Audit** : `tenant_id` résolu tracé dans `chewing_metrics`.
//! - **Mission B** : allow-list filtre le fan-out, refus si < quorum.
//! - **Speculative** : allow-list valide drafter + verifier (fail-closed).
//! - **Isolation circuit** : `slm_circuit_for` résout `{tenant}:{robot}`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    circuit_key, run_mission_a, run_mission_b, run_speculative, Coordinator, CoordinatorClients,
    MissionARequest, MissionBRequest, MockLlmProvider, PolicyRegistry, ProviderRegistry,
    SpeculativeRequest, TenantPolicies,
};
use efe_nano_app_coordinator::clients::router::RouterClient;
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};
use efe_nano_app_coordinator::SlmCircuitBreaker;
use uuid::Uuid;

// ── Helpers ───────────────────────────────────────────────────────────────

/// `LlmMetadata` paramétrable : coût (µ¢/M input=output) + capability pour
/// FactualQa. Latence basse, success 1.0.
fn meta_cost_cap(cost_microcents_per_m: u64, capability: f32) -> LlmMetadata {
    let mut capability_scores = HashMap::new();
    capability_scores.insert("FACTUAL_QA".to_string(), capability);
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: cost_microcents_per_m,
        cost_output_per_million_microcents: cost_microcents_per_m,
        p50_latency_ms: 100,
        p99_latency_ms: 500,
        success_rate: 1.0,
        capability_scores,
        circuit_open: false,
    }
}

/// Budget non-contraignant (large), à resserrer éventuellement par tenant cap.
fn budget_generous() -> Budget {
    Budget {
        max_cost_cents: 0,
        max_cost_microcents: 1_000_000, // 10 ¢ — large
        max_latency_ms: 10_000,
        min_quality_score: 0.0,
    }
}

/// Coordinator avec un **router global** à poids custom (pour activer le filtre
/// budgétaire sans dépendre d'une table de scoring par-requête).
fn coord_with_router_and_policies(
    providers: &[(&str, &str)],
    weights: (f32, f32, f32),
    policies: PolicyRegistry,
) -> Coordinator {
    let reg = ProviderRegistry::new();
    for (id, resp) in providers {
        reg.register(Arc::new(MockLlmProvider::new(*id).with_response(*resp)));
    }
    let clients = CoordinatorClients {
        router: Some(Arc::new(RouterClient::with_weights(weights.0, weights.1, weights.2))),
        ..CoordinatorClients::default()
    };
    Coordinator::builder()
        .registry(reg)
        .clients(clients)
        .policies(policies)
        .build()
        .expect("coordinator builds")
}

/// `MissionARequest` factuelle avec router global (pas de table par-requête),
/// budget large.
fn mission_a_req(available: &[&str], metas: HashMap<String, LlmMetadata>) -> MissionARequest {
    MissionARequest {
        user_text: "Quelle est la capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: available.iter().map(|s| s.to_string()).collect(),
        llm_metadata: metas,
        budget: budget_generous(),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
        tenant_id: None,
    }
}

// =====================================================================
// #10 — Backward-compat : sans policy ⇒ comportement R3.S4
// =====================================================================

#[tokio::test]
async fn tenant_policies_default_falls_back_to_global() {
    // Pas de policies + tenant_id None → default_tenant global, aucun filtrage.
    let coord = coord_with_router_and_policies(
        &[("claude", "PARIS-claude")],
        (1.0, 0.0, 1.0),
        PolicyRegistry::new(),
    );
    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta_cost_cap(1_000, 0.9));
    let req = mission_a_req(&["claude"], metas);

    let resp = run_mission_a(&coord, req).await.expect("mission A OK");
    assert_eq!(resp.llm_used, "claude");
    assert_eq!(resp.final_text, "PARIS-claude");
    // tenant_id tracé = default global (nil), pas de filtrage.
    let cm = resp.chewing_metrics.as_ref().unwrap();
    assert_eq!(cm.tenant_id, Some(Uuid::nil()));
    assert_eq!(cm.tenant_llms_filtered, None, "pas de policy → None");
}

// =====================================================================
// #11 — Divergence via allow-list : 2 tenants → 2 routings + 2 coûts
// =====================================================================

#[tokio::test]
async fn tenant_a_vs_tenant_b_diverge_via_allow_list() {
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    // A ne peut utiliser que claude ; B que gemini.
    let policies = PolicyRegistry::new()
        .with(TenantPolicies {
            allowed_llms: Some(vec!["claude".into()]),
            ..TenantPolicies::open(tenant_a)
        })
        .with(TenantPolicies {
            allowed_llms: Some(vec!["gemini".into()]),
            ..TenantPolicies::open(tenant_b)
        });
    let coord = coord_with_router_and_policies(
        &[("claude", "from-claude"), ("gemini", "from-gemini")],
        (1.0, 0.0, 1.0),
        policies,
    );

    let mut metas = HashMap::new();
    // claude cher, gemini cheap → coûts observés différents selon le tenant.
    metas.insert("claude".to_string(), meta_cost_cap(50_000, 0.9));
    metas.insert("gemini".to_string(), meta_cost_cap(1_000, 0.9));

    // Tenant A → claude.
    let mut req_a = mission_a_req(&["claude", "gemini"], metas.clone());
    req_a.tenant_id = Some(tenant_a);
    let resp_a = run_mission_a(&coord, req_a).await.expect("A OK");

    // Tenant B → gemini.
    let mut req_b = mission_a_req(&["claude", "gemini"], metas);
    req_b.tenant_id = Some(tenant_b);
    let resp_b = run_mission_a(&coord, req_b).await.expect("B OK");

    assert_eq!(resp_a.llm_used, "claude", "A allow-list = claude");
    assert_eq!(resp_b.llm_used, "gemini", "B allow-list = gemini");
    assert_ne!(resp_a.llm_used, resp_b.llm_used, "2 tenants → 2 routings");
    assert_eq!(resp_a.final_text, "from-claude");
    assert_eq!(resp_b.final_text, "from-gemini");
    // Chaque tenant a filtré 1 LLM (l'autre).
    assert_eq!(
        resp_a.chewing_metrics.as_ref().unwrap().tenant_llms_filtered,
        Some(1)
    );
    assert_eq!(
        resp_b.chewing_metrics.as_ref().unwrap().tenant_llms_filtered,
        Some(1)
    );
}

// =====================================================================
// #14 — Plafond de coût : le cap tenant exclut le LLM premium (filtre router)
// =====================================================================

#[tokio::test]
async fn tenant_cost_cap_excludes_premium_llm() {
    let tenant_open = Uuid::now_v7();
    let tenant_capped = Uuid::now_v7();
    let policies = PolicyRegistry::new()
        .with(TenantPolicies::open(tenant_open)) // pas de cap
        .with(TenantPolicies {
            // Coût estimé (≈ 509 tokens) : premium ≈ 5090 µ¢, cheap ≈ 10 µ¢.
            // Cap 1000 µ¢ → exclut premium (filtre router), garde cheap.
            max_cost_microcents_per_query: Some(1_000),
            ..TenantPolicies::open(tenant_capped)
        });
    // Poids : quality 1.0, latency 0, cost 0.5 → sans cap, premium (cap 1.0)
    // l'emporte sur cheap (cap 0.3) malgré son coût.
    let coord = coord_with_router_and_policies(
        &[("premium", "PREMIUM"), ("cheap", "CHEAP")],
        (1.0, 0.0, 0.5),
        policies,
    );

    let mut metas = HashMap::new();
    // premium : capability haute, coût élevé (≈ 5090 µ¢). cheap : capability
    // basse, coût bas (≈ 10 µ¢). Le cap tenant (1000 µ¢) exclut premium.
    metas.insert("premium".to_string(), meta_cost_cap(10_000_000, 1.0));
    metas.insert("cheap".to_string(), meta_cost_cap(20_000, 0.3));

    // Tenant open : premium gagne (pas de cap, qualité dominante).
    let mut req_open = mission_a_req(&["premium", "cheap"], metas.clone());
    req_open.tenant_id = Some(tenant_open);
    let resp_open = run_mission_a(&coord, req_open).await.expect("open OK");

    // Tenant capped : premium (50 000 µ¢) > cap (10 000) → filtré → cheap.
    let mut req_capped = mission_a_req(&["premium", "cheap"], metas);
    req_capped.tenant_id = Some(tenant_capped);
    let resp_capped = run_mission_a(&coord, req_capped).await.expect("capped OK");

    assert_eq!(resp_open.llm_used, "premium", "sans cap → premium (qualité)");
    assert_eq!(
        resp_capped.llm_used, "cheap",
        "cap exclut premium par le filtre budgétaire → cheap"
    );
    assert_ne!(resp_open.llm_used, resp_capped.llm_used, "cap → routing différent");
}

// =====================================================================
// #12 — Allow-list filtre la liste candidate (compte tracé)
// =====================================================================

#[tokio::test]
async fn tenant_allowed_llms_filters_available() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec!["gemini".into()]),
        ..TenantPolicies::open(tenant)
    });
    let coord = coord_with_router_and_policies(
        &[("claude", "C"), ("gemini", "G")],
        (1.0, 0.0, 1.0),
        policies,
    );
    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta_cost_cap(1_000, 0.9));
    metas.insert("gemini".to_string(), meta_cost_cap(1_000, 0.9));

    let mut req = mission_a_req(&["claude", "gemini"], metas);
    req.tenant_id = Some(tenant);
    let resp = run_mission_a(&coord, req).await.expect("OK");

    assert_eq!(resp.llm_used, "gemini", "seul gemini permis");
    assert_eq!(
        resp.chewing_metrics.as_ref().unwrap().tenant_llms_filtered,
        Some(1),
        "claude retiré par l'allow-list"
    );
}

// =====================================================================
// #13 — Deny-all (Some([])) ⇒ fail-closed
// =====================================================================

#[tokio::test]
async fn tenant_deny_all_fails_closed() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec![]), // deny-all explicite
        ..TenantPolicies::open(tenant)
    });
    let coord =
        coord_with_router_and_policies(&[("claude", "C")], (1.0, 0.0, 1.0), policies);
    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta_cost_cap(1_000, 0.9));

    let mut req = mission_a_req(&["claude"], metas);
    req.tenant_id = Some(tenant);
    let r = run_mission_a(&coord, req).await;
    assert!(r.is_err(), "deny-all → erreur (pas de fuite)");
}

// =====================================================================
// #15 — tenant_id résolu tracé dans chewing_metrics (fin fiction L5-a)
// =====================================================================

#[tokio::test]
async fn tenant_id_propagated_to_metrics() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies::open(tenant));
    let coord =
        coord_with_router_and_policies(&[("claude", "C")], (1.0, 0.0, 1.0), policies);
    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta_cost_cap(1_000, 0.9));

    let mut req = mission_a_req(&["claude"], metas);
    req.tenant_id = Some(tenant);
    let resp = run_mission_a(&coord, req).await.expect("OK");
    assert_eq!(
        resp.chewing_metrics.as_ref().unwrap().tenant_id,
        Some(tenant),
        "le tenant résolu (≠ default) est tracé"
    );
}

// =====================================================================
// #16 — Mission B : allow-list sous quorum ⇒ erreur ; au-dessus ⇒ filtre
// =====================================================================

fn mission_b_req(llms: &[&str], tenant: Option<Uuid>) -> MissionBRequest {
    let mut metas = HashMap::new();
    for id in llms {
        metas.insert(id.to_string(), meta_cost_cap(1_000, 0.9));
    }
    MissionBRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: llms.iter().map(|s| s.to_string()).collect(),
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        tenant_id: tenant,
    }
}

#[tokio::test]
async fn mission_b_allow_list_below_quorum_errors() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec!["a".into()]), // ne laisse qu'1 LLM
        ..TenantPolicies::open(tenant)
    });
    let reg = ProviderRegistry::new();
    for id in ["a", "b", "c"] {
        reg.register(Arc::new(MockLlmProvider::new(id).with_response("r")));
    }
    let coord = Coordinator::builder()
        .registry(reg)
        .policies(policies)
        .build()
        .unwrap();

    let req = mission_b_req(&["a", "b", "c"], Some(tenant));
    let r = run_mission_b(&coord, req).await;
    assert!(r.is_err(), "allow-list laisse < 2 LLMs → vote impossible → erreur");
}

#[tokio::test]
async fn mission_b_allow_list_filters_fanout() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec!["a".into(), "b".into()]), // retire c
        ..TenantPolicies::open(tenant)
    });
    let reg = ProviderRegistry::new();
    for id in ["a", "b", "c"] {
        reg.register(Arc::new(MockLlmProvider::new(id).with_response("Paris")));
    }
    let coord = Coordinator::builder()
        .registry(reg)
        .policies(policies)
        .build()
        .unwrap();

    let req = mission_b_req(&["a", "b", "c"], Some(tenant));
    let resp = run_mission_b(&coord, req).await.expect("B OK avec 2 LLMs");
    // c retiré → 1 LLM filtré.
    assert_eq!(
        resp.chewing_metrics.as_ref().unwrap().tenant_llms_filtered,
        Some(1)
    );
    // Le winner doit être l'un des LLMs permis (jamais "c").
    assert_ne!(resp.winner_llm, "c");
}

// =====================================================================
// #18 — Speculative : allow-list valide drafter + verifier (fail-closed)
// =====================================================================

fn spec_req(drafter: &str, verifier: &str, tenant: Option<Uuid>) -> SpeculativeRequest {
    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Quelle est la capitale de la France ?".into(),
        TaskClass::FactualQa,
        drafter.into(),
        verifier.into(),
    );
    req.max_words_for_quick_accept = 10;
    req.tenant_id = tenant;
    req
}

#[tokio::test]
async fn speculative_allow_list_forbids_drafter_fails_closed() {
    let tenant = Uuid::now_v7();
    // allow-list permet seulement le verifier, pas le drafter.
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec!["verifier".into()]),
        ..TenantPolicies::open(tenant)
    });
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("verifier").with_response("Paris")));
    let coord = Coordinator::builder()
        .registry(reg)
        .policies(policies)
        .build()
        .unwrap();

    let req = spec_req("drafter", "verifier", Some(tenant));
    let r = run_speculative(&coord, req).await;
    assert!(r.is_err(), "drafter interdit par l'allow-list → fail-closed");
}

#[tokio::test]
async fn speculative_allowed_drafter_verifier_runs_and_traces_tenant() {
    let tenant = Uuid::now_v7();
    let policies = PolicyRegistry::new().with(TenantPolicies {
        allowed_llms: Some(vec!["drafter".into(), "verifier".into()]),
        ..TenantPolicies::open(tenant)
    });
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("verifier").with_response("Paris is capital")));
    let coord = Coordinator::builder()
        .registry(reg)
        .policies(policies)
        .build()
        .unwrap();

    let req = spec_req("drafter", "verifier", Some(tenant));
    let resp = run_speculative(&coord, req).await.expect("Spec OK");
    assert_eq!(
        resp.chewing_metrics.as_ref().unwrap().tenant_id,
        Some(tenant)
    );
}

// =====================================================================
// #17 — Isolation circuit : slm_circuit_for résout {tenant}:{robot}
// =====================================================================

#[tokio::test]
async fn tenant_circuit_isolation_via_slm_circuit_for() {
    let t1 = Uuid::now_v7();
    let t2 = Uuid::now_v7();
    // Circuits keyed par (tenant, robot). t1:analyzer pré-ouvert, t2:analyzer fermé.
    let mut circuits = HashMap::new();
    let open_cb = SlmCircuitBreaker::new(1, Duration::from_secs(3600));
    open_cb.record_failure(); // → Open (seuil 1)
    circuits.insert(circuit_key(t1, "analyzer", true), open_cb);
    circuits.insert(
        circuit_key(t2, "analyzer", true),
        SlmCircuitBreaker::new(3, Duration::from_secs(10)),
    );
    // Les deux tenants ont une policy (has_policy = true → lookup keyed).
    let policies = PolicyRegistry::new()
        .with(TenantPolicies::open(t1))
        .with(TenantPolicies::open(t2));
    let coord = Coordinator::builder()
        .slm_circuits(circuits)
        .policies(policies)
        .build()
        .unwrap();

    // t1 voit son circuit OUVERT, t2 voit le sien FERMÉ — isolation prouvée.
    let c1 = coord.slm_circuit_for(t1, "analyzer", true).expect("t1 circuit");
    let c2 = coord.slm_circuit_for(t2, "analyzer", true).expect("t2 circuit");
    assert!(c1.is_open(), "circuit de t1 ouvert");
    assert!(!c2.is_open(), "circuit de t2 fermé (blast radius isolé)");
}

#[tokio::test]
async fn slm_circuit_for_falls_back_to_shared_key_when_no_tenant_circuit() {
    // Mono-tenant R3.S2 : circuit keyed par robot seul.
    let mut circuits = HashMap::new();
    circuits.insert(
        "analyzer".to_string(),
        SlmCircuitBreaker::new(3, Duration::from_secs(10)),
    );
    let coord = Coordinator::builder().slm_circuits(circuits).build().unwrap();
    let t = Uuid::now_v7();
    // has_policy=true mais pas de clé "{t}:analyzer" → fallback "analyzer".
    assert!(
        coord.slm_circuit_for(t, "analyzer", true).is_some(),
        "dégradation propre vers la clé partagée"
    );
    // has_policy=false → clé robot directement.
    assert!(coord.slm_circuit_for(t, "analyzer", false).is_some());
}
