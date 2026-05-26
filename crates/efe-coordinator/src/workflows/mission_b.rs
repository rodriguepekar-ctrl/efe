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
use crate::request::{FactCheck, MissionBRequest, MissionBResponse};
use crate::vote::{Ballot, VoteEngine};

/// Exécute la Mission B : pré-mâcher une fois, fan-out vers N LLMs en
/// parallèle, agréger via vote pondéré, fact-check le winner.
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
            let c = CompressorInput {
                text: req.user_text.clone(),
                target_words: 100,
                must_preserve: vec![],
                style: CompressionStyle::Extractive,
            };
            compressor
                .invoke(c, &ctx)
                .await
                .map(|o| o.summary)
                .unwrap_or_else(|_| req.user_text.clone())
        } else {
            req.user_text.clone()
        }
    } else {
        req.user_text.clone()
    };

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
    let responses: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
    if responses.is_empty() {
        let _ = coord
            .bus
            .publish(StreamEvent::SwarmFailed {
                swarm_id,
                mission: MissionTag::MissionB,
                stage: "llm_fanout".into(),
                error: "all providers failed".into(),
                ts: Utc::now(),
            })
            .await;
        return Err(CoordError::LlmProvider {
            provider: "*".into(),
            reason: "all providers failed".into(),
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
            if let Ok(out) = verifier.invoke(v_input, &ctx).await {
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
    let resp = MissionBResponse {
        final_text: outcome.winner_text,
        winner_llm: outcome.winner,
        winner_score: outcome.winner_score,
        per_llm_scores: outcome.per_provider_scores,
        total_cost_cents: total_cost,
        latency_ms: started.elapsed().as_millis() as u64,
        verdicts,
        audit_chain_hash: audit_hash.clone(),
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
