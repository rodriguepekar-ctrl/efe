// SPDX-License-Identifier: UNLICENSED
//! **P4.S6** — Resilience Theater : `POST /v1/resilience/run`.
//!
//! Déclenche **de vraies dégradations** (pas une animation factice) et streame
//! la mission en SSE, pour que l'UI observe les overlays de résilience. Trois
//! scénarios reproductibles avec des mocks :
//!
//! - `fallback` → un LLM 429 force le **fallback chain** → `LLM_FALLBACK_TRIGGERED`,
//!   puis la mission réussit via le LLM de secours (dégradation gracieuse).
//! - `drafter_bypass` → le drafter spéculatif échoue → **bypass** vers le
//!   verifier → `DRAFTER_BYPASSED`, réponse produite quand même.
//! - `noise_rejection` → le drafter produit du bruit (`"La"`, 0 mot de contenu)
//!   → rejeté du hot path → `DRAFT_REJECTED_AS_NOISE`, slow path → vraie réponse.
//!
//! (`SLM_CIRCUIT_OPENED` requiert des daemons SLM réels — hors mode mock.)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use efe_coordinator::{
    Coordinator, MetricsRegistry, MissionARequest, MockLlmProvider, ProviderRegistry,
    SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

use crate::handlers::mission_sse;

/// Scénario de dégradation à déclencher.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResilienceScenario {
    /// LLM sélectionné 429 → fallback chain.
    Fallback,
    /// Drafter spéculatif en échec → bypass verifier.
    DrafterBypass,
    /// Drafter produit du bruit → rejet hot path.
    NoiseRejection,
}

/// Corps de `POST /v1/resilience/run`.
#[derive(Debug, Clone, Deserialize)]
pub struct ResilienceDto {
    pub scenario: ResilienceScenario,
    #[serde(default)]
    pub user_text: Option<String>,
}

/// `POST /v1/resilience/run` — déclenche un scénario et streame la mission (SSE).
pub async fn resilience_run(Json(dto): Json<ResilienceDto>) -> Response {
    let user_text = dto
        .user_text
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Quelle est la capitale de la France ?".to_string());

    match dto.scenario {
        ResilienceScenario::Fallback => {
            let coord = fallback_coordinator();
            let stream = coord.run_mission_a_streamed(fallback_request(&user_text));
            mission_sse(stream).into_response()
        }
        ResilienceScenario::DrafterBypass => {
            // drafter en échec (run() Err) → bypass.
            let coord = speculative_coordinator("drafter-down", None);
            let req = SpeculativeRequest::with_drafter_and_verifier(
                user_text,
                TaskClass::FactualQa,
                "drafter-down".to_string(),
                "verifier-ok".to_string(),
            );
            let stream = coord.run_speculative_streamed(req);
            mission_sse(stream).into_response()
        }
        ResilienceScenario::NoiseRejection => {
            // drafter produit "La" (0 mot de contenu) → ONLY_STOPWORDS → rejet.
            let coord = speculative_coordinator("drafter-noise", Some("La"));
            let req = SpeculativeRequest::with_drafter_and_verifier(
                user_text,
                TaskClass::FactualQa,
                "drafter-noise".to_string(),
                "verifier-ok".to_string(),
            );
            let stream = coord.run_speculative_streamed(req);
            mission_sse(stream).into_response()
        }
    }
}

/// Métadonnées LLM explicites (coût/capability) pour piloter le routing.
fn meta_for(cost_microcents_per_m: u64, capability: f32) -> LlmMetadata {
    let mut caps = HashMap::new();
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
}

/// Coordinator scénario `fallback` : `flaky` (429 systématique) + `healthy`.
fn fallback_coordinator() -> Arc<Coordinator> {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("flaky").with_error("HTTP 429 (chaos)"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("healthy").with_response("Paris (servi via fallback)."),
    ));
    Arc::new(
        Coordinator::builder()
            .registry(reg)
            .metrics(Arc::new(MetricsRegistry::new()))
            .build()
            .expect("fallback coordinator"),
    )
}

/// Requête `fallback` : `flaky` cheap+capable → le router le SÉLECTIONNE en
/// premier ; il 429 → fallback sur `healthy`.
fn fallback_request(user_text: &str) -> MissionARequest {
    let mut metas = HashMap::new();
    metas.insert("flaky".to_string(), meta_for(1_000, 1.0));
    metas.insert("healthy".to_string(), meta_for(5_000, 0.3));
    MissionARequest {
        user_text: user_text.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["flaky".to_string(), "healthy".to_string()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 0,
            max_cost_microcents: 1_000_000_000,
            max_latency_ms: 30_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(10),
        task_class_scoring_table: None,
        tenant_id: None,
    }
}

/// Coordinator spéculatif : drafter (réponse `Some` ou échec `None`) + verifier sain.
fn speculative_coordinator(drafter_id: &str, drafter_response: Option<&str>) -> Arc<Coordinator> {
    let reg = ProviderRegistry::new();
    let drafter = match drafter_response {
        Some(text) => MockLlmProvider::new(drafter_id).with_response(text),
        None => MockLlmProvider::new(drafter_id).with_error("drafter down (chaos)"),
    };
    reg.register(Arc::new(drafter));
    reg.register(Arc::new(
        MockLlmProvider::new("verifier-ok").with_response("Paris (réponse du verifier)."),
    ));
    Arc::new(
        Coordinator::builder()
            .registry(reg)
            .metrics(Arc::new(MetricsRegistry::new()))
            .build()
            .expect("speculative coordinator"),
    )
}
