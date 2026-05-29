// SPDX-License-Identifier: UNLICENSED
//! **R3.S4 (fix L1/L3)** — Tests d'intégration du drafter SLM local dans
//! Mission Speculative.
//!
//! Ces tests valident le *wiring* ajouté en R3.S4 phase 9 dans
//! `run_speculative` :
//!
//! 1. **`spec_drafter_is_local`** — le flag de segmentation break-even est
//!    positionné selon la convention d'ID (`drafter_llm.starts_with("local")`).
//! 2. **Circuit drafter** — quand un `SlmCircuitBreaker` est enregistré sous
//!    l'ID du drafter et qu'il est ouvert, le Speculator bypasse le drafter
//!    (llama-server local mort) et tombe en verifier-only — sans propager
//!    l'erreur, la mission produit quand même la bonne réponse.
//! 3. **Circuit fermé** — le hot path local fonctionne normalement et le
//!    flag local est tracé.
//!
//! Note : ces tests N'UTILISENT PAS la feature `local-slm`. Le wiring
//! (`with_drafter_circuit_opt` + `spec_drafter_is_local`) n'est pas
//! feature-gated ; seul le `LocalSlmProvider` concret l'est. On simule donc
//! le drafter local avec un `MockLlmProvider` enregistré sous un ID préfixé
//! `local` — ce qui suffit à exercer toute la logique de coordination.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_speculative, Coordinator, MockLlmProvider, ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};
use efe_nano_app_coordinator::SlmCircuitBreaker;

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Construit une requête Speculative avec hot path actif (≤ 10 mots) et le
/// gate de bruit R3.S3 actif.
fn spec_req(drafter_llm: &str, verifier_llm: &str) -> SpeculativeRequest {
    SpeculativeRequest {
        user_text: "Quelle est la capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: verifier_llm.into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 1000,
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 64,
        max_total_latency: Duration::from_secs(2),
        drafter_llm: drafter_llm.to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10,
        reject_draft_noise: true,
        min_words_for_quick_accept: 1,
    }
}

/// Coordinator avec un drafter + verifier mock et, optionnellement, un
/// circuit enregistré sous l'ID du drafter.
fn coord_with(
    drafter: MockLlmProvider,
    verifier: MockLlmProvider,
    drafter_circuit: Option<(String, SlmCircuitBreaker)>,
) -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(drafter));
    reg.register(Arc::new(verifier));
    let mut builder = Coordinator::builder().registry(reg);
    if let Some((id, cb)) = drafter_circuit {
        let mut circuits = HashMap::new();
        circuits.insert(id, cb);
        builder = builder.slm_circuits(circuits);
    }
    builder.build().expect("coordinator builds")
}

// =====================================================================
// Test 1 — spec_drafter_is_local = Some(true) pour un drafter local
// =====================================================================

/// **R3.S4** : un drafter enregistré sous un ID préfixé `local` positionne
/// `spec_drafter_is_local = Some(true)` dans les ChewingMetrics. C'est l'axe
/// de segmentation de l'analyse break-even (hot-path-rate local vs cloud).
#[tokio::test]
async fn spec_drafter_is_local_true_for_local_prefixed_id() {
    let coord = coord_with(
        MockLlmProvider::new("local-slm").with_response("Paris"),
        MockLlmProvider::new("claude").with_response("Paris is the capital"),
        None,
    );
    let resp = run_speculative(&coord, spec_req("local-slm", "claude"))
        .await
        .expect("speculative OK");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics present");
    assert_eq!(
        cm.spec_drafter_is_local,
        Some(true),
        "drafter ID préfixé `local` → flag local true"
    );
}

// =====================================================================
// Test 2 — spec_drafter_is_local = Some(false) pour un drafter cloud
// =====================================================================

/// **R3.S4** : un drafter cloud (ID non préfixé `local`) positionne
/// `spec_drafter_is_local = Some(false)`. Garantit que le flag discrimine
/// honnêtement et n'est pas systématiquement vrai.
#[tokio::test]
async fn spec_drafter_is_local_false_for_cloud_drafter() {
    let coord = coord_with(
        MockLlmProvider::new("gemini-flash").with_response("Paris"),
        MockLlmProvider::new("claude").with_response("Paris is the capital"),
        None,
    );
    let resp = run_speculative(&coord, spec_req("gemini-flash", "claude"))
        .await
        .expect("speculative OK");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics present");
    assert_eq!(
        cm.spec_drafter_is_local,
        Some(false),
        "drafter cloud → flag local false"
    );
}

// =====================================================================
// Test 3 — circuit drafter OUVERT → verifier-only fallback
// =====================================================================

/// **R3.S4** : un circuit enregistré sous l'ID du drafter et déjà ouvert
/// (llama-server local mort) force le bypass du drafter. Le drafter mock
/// FAIL (`with_error`) — s'il était appelé, la mission échouerait. Le fait
/// que la mission réussisse avec la réponse du verifier prouve le skip.
#[tokio::test]
async fn open_drafter_circuit_forces_verifier_only_fallback() {
    let cb = SlmCircuitBreaker::new(1, Duration::from_secs(3600));
    cb.record_failure(); // seuil = 1 → Open immédiat, reset dans 1 h.
    assert!(cb.is_open(), "pré-condition : circuit ouvert");

    let coord = coord_with(
        // Drafter local en panne : ne DOIT pas être atteint.
        MockLlmProvider::new("local-slm").with_error("llama-server down"),
        MockLlmProvider::new("claude").with_response("Paris"),
        Some(("local-slm".to_string(), cb)),
    );

    let resp = run_speculative(&coord, spec_req("local-slm", "claude"))
        .await
        .expect("speculative OK malgré drafter mort (fallback verifier-only)");

    assert!(
        !resp.verifier_skipped,
        "circuit ouvert → bypass drafter → verifier appelé (slow path)"
    );
    assert!(
        !resp.draft_accepted,
        "aucun draft produit → rien à accepter"
    );
    assert_eq!(resp.final_text, "Paris", "le verifier produit la réponse");
    assert!(
        resp.draft_text.is_empty(),
        "drafter bypassé → draft_text vide, got: {:?}",
        resp.draft_text
    );
    // Le flag local reste tracé même en fallback (l'ID du drafter ne change pas).
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics present");
    assert_eq!(cm.spec_drafter_is_local, Some(true));
}

// =====================================================================
// Test 4 — circuit drafter FERMÉ → hot path local normal
// =====================================================================

/// **R3.S4** : un circuit fermé enregistré sous l'ID du drafter local
/// laisse passer le hot path. Le drafter local répond proprement « Paris »
/// (≤ 10 mots, non-bruit) → le verifier est skippé (vrai savings). Vérifie
/// que la présence d'un circuit n'introduit pas de régression sur le hot
/// path nominal.
#[tokio::test]
async fn closed_drafter_circuit_allows_local_hot_path() {
    let cb = SlmCircuitBreaker::new(3, Duration::from_secs(10)); // fermé
    assert!(!cb.is_open(), "pré-condition : circuit fermé");

    let coord = coord_with(
        MockLlmProvider::new("local-slm").with_response("Paris"),
        MockLlmProvider::new("claude").with_response("Paris is the capital"),
        Some(("local-slm".to_string(), cb)),
    );

    let resp = run_speculative(&coord, spec_req("local-slm", "claude"))
        .await
        .expect("speculative OK");

    assert!(
        resp.verifier_skipped,
        "drafter local « Paris » propre + circuit fermé → hot path"
    );
    assert_eq!(resp.final_text, "Paris");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics present");
    assert_eq!(cm.spec_drafter_is_local, Some(true));
    assert_eq!(
        cm.spec_noise_rejected,
        Some(false),
        "« Paris » n'est pas du bruit"
    );
}
