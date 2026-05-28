// SPDX-License-Identifier: UNLICENSED
//! Mission speculative : drafter local + verifier gros LLM.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use crate::coordinator::Coordinator;
use crate::error::{CoordError, CoordResult};
use crate::events::{MissionTag, StreamEvent};
use crate::llm::LlmCall;
use crate::metrics::ChewingMetrics;
use crate::request::{SpeculativeRequest, SpeculativeResponse};
use crate::speculate::Speculator;

/// Estime les savings_cents si le verifier a été skippé.
///
/// Calcul : `cost(verifier_input_tokens) + cost(verifier_expected_output_tokens)`.
/// Si pas skippé → 0 (on a payé le verifier).
#[must_use]
fn estimate_savings_cents(
    verifier_skipped: bool,
    verifier_meta: &efe_nano_app_coordinator::types::LlmMetadata,
    estimated_input_tokens: u32,
    expected_output_tokens: u32,
) -> u32 {
    if !verifier_skipped {
        return 0;
    }
    let in_c = (estimated_input_tokens as u64)
        * (verifier_meta.cost_input_per_million as u64)
        / 1_000_000;
    let out_c = (expected_output_tokens as u64)
        * (verifier_meta.cost_output_per_million as u64)
        / 1_000_000;
    (in_c + out_c) as u32
}

/// Exécute la Mission speculative.
///
/// **Sprint S7 R2** — Le drafter est trouvé dans le registre sous l'ID
/// `req.drafter_llm` (défaut `"drafter"` pour backward-compat). Le verifier
/// est récupéré sous `req.verifier_llm`.
///
/// Configuration typique production S7 :
/// - `drafter_llm = "gemini_gemini-2.5-flash"` (cheap LLM ~0.075 $/M)
/// - `verifier_llm = "claude_claude-sonnet-4-6"` (premium LLM ~3 $/M)
/// - `accept_threshold = 0.7` (Jaccard slow path)
/// - `max_words_for_quick_accept = 10` (S7 hot path heuristique)
///
/// Le Speculator décide hot/slow path :
/// - HOT PATH (~80 % FactualQa) : drafter ≤ 10 mots → accept sans verifier
/// - SLOW PATH (~20 %) : drafter > 10 mots → verifier appelé pour Jaccard arbitrage
pub async fn run_speculative(
    coord: &Coordinator,
    req: SpeculativeRequest,
) -> CoordResult<SpeculativeResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();

    // ── Initialisation chewing_metrics ──
    let mut chewing_metrics = ChewingMetrics {
        input_tokens_raw: ChewingMetrics::estimate_tokens(&req.user_text),
        input_tokens_compressed: ChewingMetrics::estimate_tokens(&req.user_text),
        compression_ratio: 1.0, // pas de compressor en mission spec
        savings_tokens: 0,
        savings_ratio: 0.0,
        ..ChewingMetrics::default()
    };

    let _ = coord
        .bus
        .publish(StreamEvent::SwarmStarted {
            swarm_id,
            mission: MissionTag::Speculative,
            user_text: req.user_text.clone(),
            task_class: req.task_class,
            ts: Utc::now(),
        })
        .await;

    // ── Récupération des providers (drafter + verifier) ──
    //
    // S7 R2 : `drafter_llm` est dynamique. En prod, c'est typiquement
    // l'ID d'un provider Gemini (cheap LLM cloud) ou plus tard un SLM
    // local (S9). En tests offline, on enregistre un MockLlmProvider sous
    // le même ID.
    let drafter = coord.bridge.registry().get(&req.drafter_llm).ok_or_else(|| {
        CoordError::LlmProvider {
            provider: req.drafter_llm.clone(),
            reason: "drafter provider not registered".into(),
        }
    })?;
    let verifier = coord.bridge.registry().get(&req.verifier_llm).ok_or_else(|| {
        CoordError::LlmProvider {
            provider: req.verifier_llm.clone(),
            reason: "verifier provider not registered".into(),
        }
    })?;

    // ── Speculator avec hot-path activé via `max_words_for_quick_accept` ──
    let speculator = Arc::new(
        Speculator::new(drafter, verifier, req.accept_threshold)
            .with_quick_accept(req.max_words_for_quick_accept)
            .with_bus(Arc::clone(&coord.bus), swarm_id),
    );
    let llm_call = LlmCall {
        prompt: req.user_text.clone(),
        max_tokens: req.max_draft_tokens,
        temperature: 0.7,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("swarm:{swarm_id}:speculative")),
    };

    let outcome = speculator.run(llm_call).await?;

    // ── Wire chewing_metrics post-speculation ──
    chewing_metrics.selected_llm = if outcome.verifier_skipped {
        req.drafter_llm.clone()
    } else {
        req.verifier_llm.clone()
    };
    chewing_metrics.llm_latency_ms = outcome.latency_ms;
    chewing_metrics.llm_tokens_generated = outcome.tokens_accepted + outcome.tokens_regenerated;
    chewing_metrics.spec_drafter_latency_ms = outcome.drafter_latency_ms;
    chewing_metrics.spec_verifier_latency_ms = outcome.verifier_latency_ms;
    chewing_metrics.verifier_skipped = Some(outcome.verifier_skipped);
    chewing_metrics.spec_savings_cents = estimate_savings_cents(
        outcome.verifier_skipped,
        &req.verifier_metadata,
        chewing_metrics.input_tokens_raw,
        req.max_draft_tokens.min(500), // expected_output estimate, cap 500
    );
    chewing_metrics.total_pipeline_latency_ms = started.elapsed().as_millis() as u64;
    chewing_metrics.router_reason = if outcome.verifier_skipped {
        format!(
            "S7 hot path : drafter {} accepted (≤ {} words), verifier {} skipped",
            req.drafter_llm, req.max_words_for_quick_accept, req.verifier_llm
        )
    } else {
        format!(
            "S7 slow path : drafter {} insufficient, verifier {} called for arbitrage",
            req.drafter_llm, req.verifier_llm
        )
    };

    // ── Emit SpeculationResolved enrichi (S7 : +verifier_skipped) ──
    let _ = coord
        .bus
        .publish(StreamEvent::SpeculationResolved {
            swarm_id,
            tokens_accepted: outcome.tokens_accepted,
            tokens_regenerated: outcome.tokens_regenerated,
            latency_saved_ms: outcome.latency_saved_ms,
            verifier_skipped: outcome.verifier_skipped,
            ts: Utc::now(),
        })
        .await;

    // ── Émet PreChewingMetrics (cohérence avec S4/S5) ──
    let _ = coord
        .bus
        .publish(StreamEvent::PreChewingMetrics {
            swarm_id,
            metrics: chewing_metrics.clone(),
            ts: Utc::now(),
        })
        .await;

    let resp = SpeculativeResponse {
        final_text: outcome.final_text,
        draft_text: outcome.draft_text,
        draft_accepted: outcome.draft_accepted,
        tokens_accepted: outcome.tokens_accepted,
        tokens_regenerated: outcome.tokens_regenerated,
        latency_saved_ms: outcome.latency_saved_ms,
        latency_ms: started.elapsed().as_millis() as u64,
        audit_chain_hash: super::audit_chain_hash(&[]),
        verifier_skipped: outcome.verifier_skipped,
        chewing_metrics: Some(chewing_metrics),
        // S7 R2 : propager le coût réel observé (drafter seul en hot path,
        // drafter + verifier en slow path).
        cost_cents: outcome.cost_cents,
    };

    let _ = coord
        .bus
        .publish(StreamEvent::SwarmFinished {
            swarm_id,
            mission: MissionTag::Speculative,
            latency_ms: resp.latency_ms,
            audit_chain_hash: resp.audit_chain_hash.clone(),
            ts: Utc::now(),
        })
        .await;

    Ok(resp)
}
