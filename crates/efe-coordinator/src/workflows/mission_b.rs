// SPDX-License-Identifier: UNLICENSED
//! Mission B : fan-out N LLMs + vote pondéré.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use efe_nano_app_coordinator::types::{
    AnalyzerInput, CompressionStyle, CompressorInput, VerifierInput, VerifierStrictness,
};
use efe_nano_app_coordinator::types::verifier::CandidateSource;
use efe_nano_app_coordinator::{
    AuditWriter, CallContext, InMemoryAuditWriter, NanoRobotClient,
};

use crate::blackboard::{BlackboardEntry, EntryKind};
use crate::coordinator::Coordinator;
use crate::error::{CoordError, CoordResult};
use crate::events::{MissionTag, StreamEvent};
use crate::llm::LlmCall;
use crate::metrics::ChewingMetrics;
use crate::request::{FactCheck, MissionBRequest, MissionBResponse};
use crate::vote::{Ballot, VoteEngine};

/// Exécute la Mission B : pré-mâcher une fois, fan-out vers N LLMs en
/// parallèle, agréger via vote pondéré, fact-check le winner.
///
/// `field_reassign_with_default` : pattern voulu — accumulation
/// incrémentale des métriques par phase (cf. mission_a.rs).
#[allow(clippy::field_reassign_with_default)]
pub async fn run_mission_b(
    coord: &Coordinator,
    req: MissionBRequest,
) -> CoordResult<MissionBResponse> {
    if req.llms.len() < 2 {
        return Err(CoordError::MissionBTooFewLlms { n: req.llms.len() });
    }

    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let mut audit_entries: Vec<Uuid> = vec![];

    // ── Sprint S5 R2 : ChewingMetrics mutualisé (1 compressor × N LLMs) ─
    // Initialisation des tokens d'entrée bruts (avant pré-mâchage).
    // Les latences par phase seront capturées incrémentalement au fil des étapes.
    let mut chewing_metrics = ChewingMetrics::default();
    chewing_metrics.input_tokens_raw = ChewingMetrics::estimate_tokens(&req.user_text);

    let _ = coord
        .bus
        .publish(StreamEvent::SwarmStarted {
            swarm_id,
            mission: MissionTag::MissionB,
            user_text: req.user_text.clone(),
            task_class: req.task_class,
            ts: Utc::now(),
        })
        .await;

    let audit = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::with_swarm(coord.config.default_tenant, swarm_id, audit.clone());

    // ── Étape 1 : pre-chew unique (analyzer + compressor) ────────────
    if let Some(analyzer) = &coord.clients.analyzer {
        let started_node = Instant::now();
        let _ = coord
            .bus
            .publish(StreamEvent::RobotStarted {
                swarm_id,
                robot_id: "analyzer".into(),
                ts: Utc::now(),
            })
            .await;
        let input = AnalyzerInput {
            text: req.user_text.clone(),
            max_entities: 20,
            source_agent: None,
        };
        let success = analyzer.invoke(input, &ctx).await.is_ok();
        let latency = started_node.elapsed().as_millis() as u64;
        chewing_metrics.analyzer_latency_ms = latency;
        let _ = coord
            .bus
            .publish(StreamEvent::RobotFinished {
                swarm_id,
                robot_id: "analyzer".into(),
                success,
                latency_ms: latency,
                error: None,
                ts: Utc::now(),
            })
            .await;
    }

    let prompt_for_llms = if let Some(compressor) = &coord.clients.compressor {
        if req.user_text.len() > 500 {
            // S5 R2 patch : émettre RobotStarted/RobotFinished + capturer
            // l'erreur dans le bus si l'invoke échoue. Avant ce patch,
            // un compressor qui fail était SWALLOW silencieusement
            // (`unwrap_or_else(|_| req.user_text.clone())` retombait sur le
            // texte original sans laisser de trace observable).
            let started_node = Instant::now();
            let _ = coord
                .bus
                .publish(StreamEvent::RobotStarted {
                    swarm_id,
                    robot_id: "compressor".into(),
                    ts: Utc::now(),
                })
                .await;
            let c = CompressorInput {
                text: req.user_text.clone(),
                // S5 R2 patch D15 : target_words=100 donnait des résumés
                // ≈ même taille que la fixture 547 chars (≈ 137 tokens raw).
                // 30 mots ≈ 120 chars ≈ 30 tokens output → ratio ≈ 0.22,
                // savings ≈ 100 tokens, ce qui se MULTIPLIE × 3 LLMs.
                target_words: 30,
                must_preserve: vec![],
                style: CompressionStyle::Extractive,
            };
            let res = compressor.invoke(c, &ctx).await;
            let latency = started_node.elapsed().as_millis() as u64;
            chewing_metrics.compressor_latency_ms = latency;
            match res {
                Ok(out) => {
                    let _ = coord
                        .bus
                        .publish(StreamEvent::RobotFinished {
                            swarm_id,
                            robot_id: "compressor".into(),
                            success: true,
                            latency_ms: latency,
                            error: None,
                            ts: Utc::now(),
                        })
                        .await;
                    let _ = coord.blackboard.deposit(BlackboardEntry::new(
                        "compressor",
                        EntryKind::CompressedText,
                        serde_json::json!({"summary": out.summary, "ratio": out.compression_ratio}),
                    ));
                    out.summary
                }
                Err(e) => {
                    let _ = coord
                        .bus
                        .publish(StreamEvent::RobotFinished {
                            swarm_id,
                            robot_id: "compressor".into(),
                            success: false,
                            latency_ms: latency,
                            error: Some(e.to_string()),
                            ts: Utc::now(),
                        })
                        .await;
                    // Fallback : on garde le user_text original (pas de compression).
                    // Le savings sera 0 mais la mission continue avec la qualité préservée.
                    req.user_text.clone()
                }
            }
        } else {
            req.user_text.clone()
        }
    } else {
        req.user_text.clone()
    };

    // S5 R2 : capture tokens compressés (envoyés en parallèle aux N LLMs).
    // Le savings de `raw - compressed` s'applique × N (1 compressor pour tous).
    chewing_metrics.input_tokens_compressed =
        ChewingMetrics::estimate_tokens(&prompt_for_llms);
    chewing_metrics.recompute_savings();

    // ── Étape 2 : fan-out vers N LLMs en streaming parallèle ─────────
    let llm_call = LlmCall {
        prompt: prompt_for_llms,
        max_tokens: 1000,
        temperature: 0.5,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("swarm:{swarm_id}:mission_b")),
    };
    let (chunk_tx, mut chunk_rx) =
        tokio::sync::mpsc::channel::<(String, crate::llm::TokenChunk)>(1024);
    let bus_for_pump = Arc::clone(&coord.bus);
    let swarm_for_pump = swarm_id;
    let pump = tokio::spawn(async move {
        while let Some((provider, chunk)) = chunk_rx.recv().await {
            let _ = bus_for_pump
                .publish(StreamEvent::LlmTokenStreamed {
                    swarm_id: swarm_for_pump,
                    provider,
                    token: chunk.token,
                    index: chunk.index,
                    ts: Utc::now(),
                })
                .await;
        }
    });
    let results = coord
        .bridge
        .stream_many(&req.llms, llm_call, chunk_tx)
        .await;
    let _ = pump.await;

    // S8 R2 : trace les LLMs ayant fail (au lieu de filter_map silencieux).
    //
    // ATTENTION ordering : `stream_many` utilise `FuturesUnordered` →
    // l'ordre des résultats reflète l'ordre d'arrivée, PAS l'ordre des
    // `req.llms`. On extrait donc le `provider_id` depuis :
    // - `LlmResponse.provider` pour les succès
    // - `CoordError::LlmProvider { provider, .. }` pour les échecs
    let mut failed_llms: Vec<(String, String)> = Vec::new();
    let mut responses: Vec<crate::llm::LlmResponse> = Vec::new();
    for r in results.into_iter() {
        match r {
            Ok(resp) => responses.push(resp),
            Err(e) => {
                // Extraction chirurgicale du provider_id depuis l'erreur.
                let llm = match &e {
                    CoordError::LlmProvider { provider, .. } => provider.clone(),
                    _ => "unknown".to_string(),
                };
                let reason = e.to_string();
                // Bus event par LLM raté.
                let _ = coord
                    .bus
                    .publish(StreamEvent::LlmFailed {
                        swarm_id,
                        provider: llm.clone(),
                        reason: reason.clone(),
                        ts: Utc::now(),
                    })
                    .await;
                failed_llms.push((llm, reason));
            }
        }
    }

    if responses.is_empty() {
        let _ = coord
            .bus
            .publish(StreamEvent::SwarmFailed {
                swarm_id,
                mission: MissionTag::MissionB,
                stage: "llm_fanout".into(),
                error: format!(
                    "all {} providers failed: {failed_llms:?}",
                    req.llms.len()
                ),
                ts: Utc::now(),
            })
            .await;
        return Err(CoordError::LlmProvider {
            provider: "*".into(),
            reason: format!("all {} providers failed", req.llms.len()),
        });
    }

    for r in &responses {
        let _ = coord
            .bus
            .publish(StreamEvent::LlmFinished {
                swarm_id,
                provider: r.provider.clone(),
                tokens_generated: r.tokens_generated,
                cost_cents: r.cost_cents,
                latency_ms: r.latency_ms,
                ts: Utc::now(),
            })
            .await;
        // Phéromone vote.
        let _ = coord.blackboard.deposit(BlackboardEntry::new(
            r.provider.clone(),
            EntryKind::Vote,
            serde_json::json!({"answer": r.text.clone(), "quality": r.self_quality}),
        ));
    }

    // S5 R2 : agrégats LLM en Mission B.
    // - latency = MAX des N (fan-out parallèle, pas somme)
    // - tokens_generated = SOMME des N (chaque LLM a produit ses tokens)
    chewing_metrics.llm_latency_ms = responses.iter().map(|r| r.latency_ms).max().unwrap_or(0);
    chewing_metrics.llm_tokens_generated =
        responses.iter().map(|r| r.tokens_generated).sum();

    // ── Étape 3 : vote pondéré ───────────────────────────────────────
    let ballots: Vec<Ballot> = responses
        .iter()
        .map(|r| Ballot {
            provider: r.provider.clone(),
            answer_text: r.text.clone(),
            self_quality: r.self_quality,
            latency_ms: r.latency_ms,
        })
        .collect();
    let quorum = req.quorum.unwrap_or_else(|| (req.llms.len() + 1) / 2);

    let success_rates: HashMap<String, f32> = req
        .llm_metadata
        .iter()
        .map(|(k, v)| (k.clone(), v.success_rate))
        .collect();
    let vote_engine = if req.vote_weights.is_empty() {
        VoteEngine::new(HashMap::new(), success_rates)
    } else {
        VoteEngine::new(req.vote_weights.clone(), success_rates)
    };
    let outcome = vote_engine.tally(&ballots, quorum)?;

    // S5 R2 : en Mission B, selected_llm = winner du vote Symphony (pas du
    // router, qui n'est pas utilisé). router_reason reste vide car le router
    // nano-robot n'a pas tourné — c'est le VoteEngine qui sélectionne.
    chewing_metrics.selected_llm = outcome.winner.clone();

    let _ = coord
        .bus
        .publish(StreamEvent::VoteResolved {
            swarm_id,
            winner: outcome.winner.clone(),
            winner_score: outcome.winner_score,
            per_llm_scores: outcome.per_provider_scores.clone(),
            ts: Utc::now(),
        })
        .await;

    // ── Étape 4 : Verifier (optionnel) ───────────────────────────────
    let mut verdicts = Vec::new();
    if !req.candidate_sources.is_empty() {
        if let Some(verifier) = &coord.clients.verifier {
            let v_input = VerifierInput {
                claim: outcome.winner_text.clone(),
                candidate_sources: req
                    .candidate_sources
                    .iter()
                    .map(|cs| CandidateSource {
                        id: cs.id.clone(),
                        text: cs.text.clone(),
                    })
                    .collect(),
                strictness: VerifierStrictness::Standard,
            };
            // S5 R2 : instrumenter latence verifier.
            let verifier_started = Instant::now();
            let res = verifier.invoke(v_input, &ctx).await;
            chewing_metrics.verifier_latency_ms =
                verifier_started.elapsed().as_millis() as u64;
            if let Ok(out) = res {
                let verdict_str = format!("{:?}", out.verdict).to_uppercase();
                let severity_str = format!("{:?}", out.severity).to_uppercase();
                let _ = coord
                    .bus
                    .publish(StreamEvent::VerdictReady {
                        swarm_id,
                        claim: outcome.winner_text.clone(),
                        verdict: verdict_str.clone(),
                        severity: severity_str.clone(),
                        best_source_id: out.best_source_id.clone(),
                        similarity: out.similarity,
                        ts: Utc::now(),
                    })
                    .await;
                verdicts.push(FactCheck {
                    claim: outcome.winner_text.clone(),
                    verdict: verdict_str,
                    severity: severity_str,
                    best_source_id: out.best_source_id,
                    similarity: out.similarity,
                });
            }
        }
    }

    // ── Audit chain ──────────────────────────────────────────────────
    let entries = audit.entries().await;
    for e in &entries {
        audit_entries.push(e.entry_id);
    }
    let audit_hash = super::audit_chain_hash(&audit_entries);

    let total_cost: u32 = responses.iter().map(|r| r.cost_cents).sum();

    // ── Sprint S5 R2 : finalisation des ChewingMetrics ────────────────
    // À ce point :
    //   - analyzer/compressor/verifier latencies déjà trackées
    //   - llm_latency_ms = max(N) déjà calculé
    //   - llm_tokens_generated = sum(N) déjà calculé
    //   - selected_llm = winner du vote déjà setté
    // On finalise les agrégats globaux et on émet l'event PRE_CHEWING_METRICS
    // juste avant SwarmFinished (cohérent avec Mission A).
    chewing_metrics.total_pipeline_latency_ms = started.elapsed().as_millis() as u64;
    chewing_metrics.recompute_total_slm_latency();

    let _ = coord
        .bus
        .publish(StreamEvent::PreChewingMetrics {
            swarm_id,
            metrics: chewing_metrics.clone(),
            ts: Utc::now(),
        })
        .await;

    // S8 R2 : calcule vote_degraded + failure_mode.
    let vote_degraded = !failed_llms.is_empty();
    let failure_mode = if vote_degraded {
        crate::failure::FailureMode::VoteDegraded {
            effective_votes: responses.len(),
            expected: req.llms.len(),
        }
    } else {
        crate::failure::FailureMode::None
    };

    let resp = MissionBResponse {
        final_text: outcome.winner_text,
        winner_llm: outcome.winner,
        winner_score: outcome.winner_score,
        per_llm_scores: outcome.per_provider_scores,
        total_cost_cents: total_cost,
        latency_ms: started.elapsed().as_millis() as u64,
        verdicts,
        audit_chain_hash: audit_hash.clone(),
        chewing_metrics: Some(chewing_metrics),
        // S8 R2 : graceful degradation tracing.
        vote_degraded,
        failed_llms,
        failure_mode,
    };

    let _ = coord
        .bus
        .publish(StreamEvent::SwarmFinished {
            swarm_id,
            mission: MissionTag::MissionB,
            latency_ms: resp.latency_ms,
            audit_chain_hash: audit_hash,
            ts: Utc::now(),
        })
        .await;

    Ok(resp)
}
