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
use crate::metering::{MissionMetricGuard, MissionRecord, MissionType};
use crate::metrics::ChewingMetrics;
use crate::request::{SpeculativeRequest, SpeculativeResponse};
use crate::speculate::{NoiseRejectionPolicy, Speculator};

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

/// **R3.S1** — Calcule les **µ¢** économisés sur le verifier en HOT PATH.
///
/// Symétrique de `estimate_savings_cents()` mais retourne en microcents pour
/// la précision sub-cent.  Utilise `_microcents_resolved()` pour gérer
/// transparently les LlmMetadata pré-R3.
#[must_use]
fn estimate_savings_microcents(
    verifier_skipped: bool,
    verifier_meta: &efe_nano_app_coordinator::types::LlmMetadata,
    estimated_input_tokens: u32,
    expected_output_tokens: u32,
) -> u64 {
    if !verifier_skipped {
        return 0;
    }
    let in_rate = verifier_meta.cost_input_microcents_resolved();
    let out_rate = verifier_meta.cost_output_microcents_resolved();
    let in_mc = (estimated_input_tokens as u64).saturating_mul(in_rate) / 1_000_000;
    let out_mc = (expected_output_tokens as u64).saturating_mul(out_rate) / 1_000_000;
    in_mc.saturating_add(out_mc)
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
    run_speculative_with_swarm_id(coord, req, Uuid::now_v7()).await
}

/// **P4.S1** — Mission speculative avec `swarm_id` injecté (abonnement SSE sans
/// course). [`run_speculative`] délègue ici avec `Uuid::now_v7()`.
pub async fn run_speculative_with_swarm_id(
    coord: &Coordinator,
    req: SpeculativeRequest,
    swarm_id: Uuid,
) -> CoordResult<SpeculativeResponse> {
    let started = Instant::now();

    // R3.S7 : garde télémétrie (cf. mission_a). Capture les erreurs (allow-list,
    // provider absent, …) via Drop ; `record_success` désarme avant Ok.
    let mut metric_guard = MissionMetricGuard::arm(coord.metrics.clone(), MissionType::Speculative);

    // ── R3.S5 : résolution multi-tenant + allow-list (Speculative) ────────
    //
    // En Speculative, l'enforcement honnête se limite à l'**allow-list** :
    // drafter ET verifier doivent être permis par la policy, sinon
    // fail-closed (pas de fuite vers un LLM hors plan). On NE resserre PAS le
    // `Budget` : `run_speculative` ne le consomme pas (aucun filtre router),
    // donc le capper serait du faux contrôle (cf. principe « zéro dead
    // config » R3.S4 / architecture §7.3, note budget).
    let (tenant_id, policy) =
        crate::multi_tenant::resolve(&coord.policies, req.tenant_id, coord.config.default_tenant);
    if let Some(p) = policy {
        if !p.permits_llm(&req.drafter_llm) || !p.permits_llm(&req.verifier_llm) {
            return Err(CoordError::Config(
                "tenant allow-list forbids drafter or verifier (Speculative)".into(),
            ));
        }
    }

    // ── Initialisation chewing_metrics ──
    let mut chewing_metrics = ChewingMetrics {
        input_tokens_raw: ChewingMetrics::estimate_tokens(&req.user_text),
        input_tokens_compressed: ChewingMetrics::estimate_tokens(&req.user_text),
        compression_ratio: 1.0, // pas de compressor en mission spec
        savings_tokens: 0,
        savings_ratio: 0.0,
        tenant_id: Some(tenant_id),
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

    // ── R3.S3 : politique de rejet de bruit ──
    //
    // `reject_draft_noise=true` (défaut) → gate actif (fix L4/D6).
    // `false` → permissive_s7 (comportement S7 exact, opt-out backward-compat).
    let noise_policy = if req.reject_draft_noise {
        NoiseRejectionPolicy {
            min_words: req.min_words_for_quick_accept,
            ..NoiseRejectionPolicy::default()
        }
    } else {
        NoiseRejectionPolicy::permissive_s7()
    };

    // ── R3.S4 : circuit-breaker du drafter ──
    //
    // Si un `SlmCircuitBreaker` est enregistré sous l'ID du drafter (cas
    // typique : drafter SLM local enregistré via
    // `CoordinatorBuilder::with_local_slm_drafter`), on le branche sur le
    // Speculator. Quand le circuit est ouvert (drafter local en panne /
    // llama-server down), `run()` bypasse le drafter et tombe directement
    // sur le verifier-only (fallback dégradé mais correct). Pour un drafter
    // cloud sans circuit configuré, `slm_circuit` renvoie `None` et le
    // Speculator conserve son comportement S7 historique.
    let drafter_circuit = coord.clients.slm_circuit(&req.drafter_llm).cloned();

    // ── Speculator avec hot-path activé via `max_words_for_quick_accept` ──
    let speculator = Arc::new(
        Speculator::new(drafter, verifier, req.accept_threshold)
            .with_quick_accept(req.max_words_for_quick_accept)
            .with_noise_policy(noise_policy)
            .with_drafter_circuit_opt(drafter_circuit)
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
    // R3.S3 : trace si le draft a été rejeté comme bruit (signal de santé
    // du drafter ; taux > 30 % ⇒ drafter mal choisi).
    chewing_metrics.spec_noise_rejected = Some(outcome.noise_signal.is_some());
    // R3.S4 : trace si le drafter était un SLM local. Sert d'axe de
    // segmentation pour l'analyse break-even (hot-path-rate local vs cloud).
    // Convention d'ID : un drafter local est enregistré sous un ID préfixé
    // `local` (cf. `LocalSlmProvider::from_env` → "local-slm" par défaut).
    chewing_metrics.spec_drafter_is_local = Some(req.drafter_llm.starts_with("local"));
    chewing_metrics.spec_savings_cents = estimate_savings_cents(
        outcome.verifier_skipped,
        &req.verifier_metadata,
        chewing_metrics.input_tokens_raw,
        req.max_draft_tokens.min(500), // expected_output estimate, cap 500
    );
    // R3.S1 : précision µ¢ — un Speculative qui économise 0.5 cent (= 500 µ¢)
    // sur 1 mission n'apparaît PAS dans `spec_savings_cents` (arrondi),
    // mais s'agrège honnêtement sur N missions.
    chewing_metrics.spec_savings_microcents = estimate_savings_microcents(
        outcome.verifier_skipped,
        &req.verifier_metadata,
        chewing_metrics.input_tokens_raw,
        req.max_draft_tokens.min(500),
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
        // R3.S1 : précision µ¢ (Gemini Flash drafter ne se voit qu'ici).
        cost_microcents: outcome.cost_microcents,
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

    // R3.S7 : enregistre la mission réussie (désarme le guard).
    metric_guard.record_success(&MissionRecord::from_speculative(&resp));
    Ok(resp)
}
