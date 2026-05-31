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
use crate::metering::{MissionMetricGuard, MissionRecord, MissionType};
use crate::metrics::ChewingMetrics;
use crate::request::{FactCheck, MissionBRequest, MissionBResponse};
use crate::vote::{Ballot, VoteEngine};
// R3.S2 (fix L2) : helpers SLM circuit guards.
use crate::workflows::slm_guard::{
    slm_guard_check, slm_guard_record_failure, slm_guard_record_success, SlmGuardOutcome,
};

/// Exécute la Mission B : pré-mâcher une fois, fan-out vers N LLMs en
/// parallèle, agréger via vote pondéré, fact-check le winner.
///
/// `field_reassign_with_default` : pattern voulu — accumulation
/// incrémentale des métriques par phase (cf. mission_a.rs).
/// Exécute la Mission B (génère un `swarm_id` frais).
///
/// **Wrapper backward-compat** → délègue à [`run_mission_b_with_swarm_id`].
pub async fn run_mission_b(
    coord: &Coordinator,
    req: MissionBRequest,
) -> CoordResult<MissionBResponse> {
    run_mission_b_with_swarm_id(coord, req, Uuid::now_v7()).await
}

/// **P4.S1** — Mission B avec `swarm_id` injecté (abonnement SSE sans course :
/// le bridge s'abonne avant le `SwarmStarted`). Cf. [`run_mission_b`].
#[allow(clippy::field_reassign_with_default)]
pub async fn run_mission_b_with_swarm_id(
    coord: &Coordinator,
    mut req: MissionBRequest,
    swarm_id: Uuid,
) -> CoordResult<MissionBResponse> {
    if req.llms.len() < 2 {
        return Err(CoordError::MissionBTooFewLlms { n: req.llms.len() });
    }

    let started = Instant::now();

    // R3.S7 : garde télémétrie (cf. mission_a). Armée après la validation
    // `< 2 LLMs` (pré-flight) ; capture les erreurs runtime via Drop.
    let mut metric_guard = MissionMetricGuard::arm(coord.metrics.clone(), MissionType::B);
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

    // ── R3.S5 : résolution multi-tenant + enforcement (Mission B) ─────────
    //
    // En Mission B : pas de router ni de `Budget`. Seule l'allow-list
    // s'applique strictement (filtre le set de fan-out `req.llms`). Le plafond
    // de coût est best-effort (tracé post-fan-out via
    // `tenant_cost_cap_exceeded`), faute de point de coupe pré-fan-out
    // (cf. architecture §7.2).
    let (tenant_id, policy) =
        crate::multi_tenant::resolve(&coord.policies, req.tenant_id, coord.config.default_tenant);
    let has_policy = policy.is_some();
    let cost_cap: Option<u64> = policy.and_then(|p| p.max_cost_microcents_per_query);
    let mut tenant_llms_filtered: Option<usize> = None;
    if let Some(p) = policy {
        let before = req.llms.len();
        req.llms = p.filter_llms(&req.llms);
        tenant_llms_filtered = Some(before - req.llms.len());
        // Mission B exige ≥ 2 LLMs pour un vote : l'allow-list ne doit pas
        // descendre sous le quorum minimal.
        if req.llms.len() < 2 {
            return Err(CoordError::Config(
                "tenant allow-list leaves < 2 LLMs for Mission B vote".into(),
            ));
        }
    }
    chewing_metrics.tenant_id = Some(tenant_id);
    chewing_metrics.tenant_llms_filtered = tenant_llms_filtered;

    let audit = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::with_swarm(tenant_id, swarm_id, audit.clone());

    // ── Étape 1 : pre-chew unique (analyzer + compressor) ────────────
    // R3.S2 (fix L2) : guards SLM circuit autour des invokes.
    if let Some(analyzer) = &coord.clients.analyzer {
        let circuit = coord.slm_circuit_for(tenant_id, "analyzer", has_policy);
        match slm_guard_check(circuit, "analyzer", &mut chewing_metrics) {
            SlmGuardOutcome::Skip => {
                // R3.S2 : circuit Open → skip analyzer.
                let _ = coord
                    .bus
                    .publish(StreamEvent::RobotFinished {
                        swarm_id,
                        robot_id: "analyzer".into(),
                        success: false,
                        latency_ms: 0,
                        error: Some("slm_circuit_open".into()),
                        ts: Utc::now(),
                    })
                    .await;
            }
            SlmGuardOutcome::Proceed => {
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
                let invoke_res = analyzer.invoke(input, &ctx).await;
                let success = invoke_res.is_ok();
                let latency = started_node.elapsed().as_millis() as u64;
                chewing_metrics.analyzer_latency_ms = latency;
                if let Err(_e) = &invoke_res {
                    // R3.S2 : échec → record_failure + auto-emit event.
                    let _opened = slm_guard_record_failure(
                        circuit,
                        "analyzer",
                        &mut chewing_metrics,
                        &coord.bus,
                        swarm_id,
                    )
                    .await;
                } else {
                    // R3.S2 : succès → reset.
                    slm_guard_record_success(circuit);
                }
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
        }
    }

    // R3.S2 (fix L2) : compressor mutualisé Mission B avec guard SLM
    // circuit. Sur fan-out N LLMs, le compressor tourne 1× → 1 seul guard
    // (cohérent avec S5 mutualisation).
    let prompt_for_llms = if let Some(compressor) = &coord.clients.compressor {
        // R5 : gate sur tokens (≥ COMPRESS_MIN_TOKENS) — cf. mission_a / should_compress.
        if super::should_compress(&req.user_text) {
            let circuit = coord.slm_circuit_for(tenant_id, "compressor", has_policy);
            match slm_guard_check(circuit, "compressor", &mut chewing_metrics) {
                SlmGuardOutcome::Skip => {
                    // R3.S2 : circuit Open → skip compressor, use raw prompt.
                    let _ = coord
                        .bus
                        .publish(StreamEvent::RobotFinished {
                            swarm_id,
                            robot_id: "compressor".into(),
                            success: false,
                            latency_ms: 0,
                            error: Some("slm_circuit_open".into()),
                            ts: Utc::now(),
                        })
                        .await;
                    req.user_text.clone()
                }
                SlmGuardOutcome::Proceed => {
                    // S5 R2 patch : émettre RobotStarted/RobotFinished + capturer
                    // l'erreur dans le bus si l'invoke échoue.
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
                        // Levier 1 : cible PROPORTIONNELLE (≈ 1 mot / 8 tokens,
                        // borné [30, 500]) — mutualisée sur N LLMs, donc d'autant
                        // plus rentable. Cf. workflows::proportional_target_words.
                        target_words: super::proportional_target_words(&req.user_text),
                        must_preserve: vec![],
                        style: CompressionStyle::Extractive,
                    };
                    let res = compressor.invoke(c, &ctx).await;
                    let latency = started_node.elapsed().as_millis() as u64;
                    chewing_metrics.compressor_latency_ms = latency;
                    match res {
                        Ok(out) => {
                            // R3.S2 : succès → reset circuit.
                            slm_guard_record_success(circuit);
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
                            // Garde anti-garbage partagé (cf. mission_a). CRITIQUE
                            // en Mission B : ce prompt part vers N LLMs en fan-out,
                            // donc un résumé dégénéré contaminerait les N réponses
                            // et le vote. On retombe sur le brut si dégénéré.
                            let (prompt, fell_back) =
                                super::final_prompt_from_compression(&out.summary, &req.user_text);
                            if fell_back {
                                chewing_metrics
                                    .failed_components
                                    .push("compressor:degenerate_output".to_string());
                            }
                            prompt
                        }
                        Err(e) => {
                            // R3.S2 : échec → record_failure + auto-emit event.
                            let _opened = slm_guard_record_failure(
                                circuit,
                                "compressor",
                                &mut chewing_metrics,
                                &coord.bus,
                                swarm_id,
                            )
                            .await;
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
                            // Fallback : on garde le user_text original.
                            req.user_text.clone()
                        }
                    }
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

    // ── Étape 4 : Verifier (R3.S2 fix L2 : guard SLM circuit + si sources) ──
    let mut verdicts = Vec::new();
    if !req.candidate_sources.is_empty() {
        if let Some(verifier) = &coord.clients.verifier {
            let circuit = coord.slm_circuit_for(tenant_id, "verifier", has_policy);
            match slm_guard_check(circuit, "verifier", &mut chewing_metrics) {
                SlmGuardOutcome::Skip => {
                    // R3.S2 : circuit Open → skip verifier, pas de verdicts.
                    let _ = coord
                        .bus
                        .publish(StreamEvent::RobotFinished {
                            swarm_id,
                            robot_id: "verifier".into(),
                            success: false,
                            latency_ms: 0,
                            error: Some("slm_circuit_open".into()),
                            ts: Utc::now(),
                        })
                        .await;
                }
                SlmGuardOutcome::Proceed => {
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
                    match res {
                        Ok(out) => {
                            // R3.S2 : succès → reset circuit.
                            slm_guard_record_success(circuit);
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
                        Err(_) => {
                            // R3.S2 : échec → record_failure + auto-emit event.
                            let _opened = slm_guard_record_failure(
                                circuit,
                                "verifier",
                                &mut chewing_metrics,
                                &coord.bus,
                                swarm_id,
                            )
                            .await;
                        }
                    }
                }
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
    // R3.S1 : agrégation µ¢ avec saturating_add — Mission B amplifie l'erreur
    // d'arrondi cent par N (= 1 par LLM), donc µ¢ critique ici.
    let total_cost_mc: u64 = responses
        .iter()
        .fold(0_u64, |acc, r| acc.saturating_add(r.cost_microcents_resolved()));

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
    // R3.S5 : plafond de coût tenant best-effort. Mission B n'a pas de point
    // de coupe pré-fan-out (pas de `Budget`/router) ; on TRACE donc un
    // dépassement post-fan-out pour alerting, sans bloquer (architecture §7.2).
    chewing_metrics.tenant_cost_cap_exceeded =
        cost_cap.map(|cap| total_cost_mc > cap);

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
        // R3.S1 : agrégation µ¢ précise.
        total_cost_microcents: total_cost_mc,
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

    // R3.S7 : enregistre la mission réussie (désarme le guard).
    metric_guard.record_success(&MissionRecord::from_mission_b(&resp));
    Ok(resp)
}
