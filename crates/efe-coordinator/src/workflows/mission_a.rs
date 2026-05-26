// SPDX-License-Identifier: UNLICENSED
//! Mission A : pré-mâcher pour 1 LLM gros.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use efe_nano_app_coordinator::types::{
    AnalyzerInput, CompressionStyle, CompressorInput, RouterInput, TaskDescription,
    VerifierInput, VerifierStrictness,
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
use crate::request::{FactCheck, MissionARequest, MissionAResponse};

/// Exécute la Mission A : pré-mâcher pour 1 LLM, retourne la réponse finale.
///
/// Étapes :
/// 1. Analyzer extrait entités + intent + résumé.
/// 2. Router choisit le LLM (avec hint task_class + budget).
/// 3. Compressor produit un summary court à injecter dans le prompt.
/// 4. Le LLM choisi est appelé en streaming.
/// 5. Le Verifier (si sources fournies) fact-check le résultat.
/// 6. La réponse finale + hash audit chain est retournée.
///
/// Tous les robots optionnels sont skippés si non fournis dans
/// `coord.clients`.
pub async fn run_mission_a(
    coord: &Coordinator,
    req: MissionARequest,
) -> CoordResult<MissionAResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let mut audit_entries: Vec<Uuid> = vec![];

    // Subscribe nos propres events sur la blackboard via mock-audit + bus.
    let _ = coord
        .bus
        .publish(StreamEvent::SwarmStarted {
            swarm_id,
            mission: MissionTag::MissionA,
            user_text: req.user_text.clone(),
            task_class: req.task_class,
            ts: Utc::now(),
        })
        .await;

    // Construit CallContext.
    let audit = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::with_swarm(coord.config.default_tenant, swarm_id, audit.clone());

    // ── Étape 1 : Analyzer ───────────────────────────────────────────
    if let Some(analyzer) = &coord.clients.analyzer {
        let _ = coord
            .bus
            .publish(StreamEvent::RobotStarted {
                swarm_id,
                robot_id: "analyzer".into(),
                ts: Utc::now(),
            })
            .await;
        let started_node = Instant::now();
        let input = AnalyzerInput {
            text: req.user_text.clone(),
            max_entities: 20,
            source_agent: None,
        };
        let res = analyzer.invoke(input, &ctx).await;
        let latency = started_node.elapsed().as_millis() as u64;
        match res {
            Ok(out) => {
                // Dépose une phéromone d'entités.
                for e in &out.entities {
                    let entry = BlackboardEntry::new(
                        "analyzer",
                        EntryKind::Entity,
                        serde_json::json!({"text": e.text, "kind": format!("{:?}", e.kind)}),
                    );
                    let entry_id = entry.entry_id;
                    let _ = coord.blackboard.deposit(entry);
                    let _ = coord
                        .bus
                        .publish(StreamEvent::PheromoneDeposited {
                            swarm_id,
                            author: "analyzer".into(),
                            kind: "ENTITY".into(),
                            entry_id,
                            ts: Utc::now(),
                        })
                        .await;
                }
                let _ = coord
                    .bus
                    .publish(StreamEvent::RobotFinished {
                        swarm_id,
                        robot_id: "analyzer".into(),
                        success: true,
                        latency_ms: latency,
                        error: None,
                        ts: Utc::now(),
                    })
                    .await;
            }
            Err(e) => {
                let _ = coord
                    .bus
                    .publish(StreamEvent::RobotFinished {
                        swarm_id,
                        robot_id: "analyzer".into(),
                        success: false,
                        latency_ms: latency,
                        error: Some(e.to_string()),
                        ts: Utc::now(),
                    })
                    .await;
                // Analyzer optionnel : on continue.
            }
        }
    }

    // ── Étape 2 : Router ──────────────────────────────────────────────
    let selected_llm = if let Some(router) = &coord.clients.router {
        let _ = coord
            .bus
            .publish(StreamEvent::RobotStarted {
                swarm_id,
                robot_id: "router".into(),
                ts: Utc::now(),
            })
            .await;
        let started_node = Instant::now();
        let r_input = RouterInput {
            task: TaskDescription {
                prompt_summary: req.user_text.chars().take(200).collect(),
                task_class: req.task_class,
                estimated_input_tokens: (req.user_text.len() / 4).max(1) as u32,
                expected_output_tokens: 500,
            },
            available_llms: req.available_llms.clone(),
            llm_metadata: req.llm_metadata.clone(),
            user_budget: req.budget.clone(),
        };
        match router.invoke(r_input, &ctx).await {
            Ok(out) => {
                let latency = started_node.elapsed().as_millis() as u64;
                let _ = coord
                    .bus
                    .publish(StreamEvent::RobotFinished {
                        swarm_id,
                        robot_id: "router".into(),
                        success: true,
                        latency_ms: latency,
                        error: None,
                        ts: Utc::now(),
                    })
                    .await;
                // Dépose phéromone.
                let _ = coord.blackboard.deposit(BlackboardEntry::new(
                    "router",
                    EntryKind::LlmSelection,
                    serde_json::json!({"selected": out.selected, "reason": out.reason}),
                ));
                out.selected
            }
            Err(e) => {
                let latency = started_node.elapsed().as_millis() as u64;
                let _ = coord
                    .bus
                    .publish(StreamEvent::RobotFinished {
                        swarm_id,
                        robot_id: "router".into(),
                        success: false,
                        latency_ms: latency,
                        error: Some(e.to_string()),
                        ts: Utc::now(),
                    })
                    .await;
                // Fallback : prend le 1er LLM disponible.
                req.available_llms
                    .first()
                    .cloned()
                    .ok_or_else(|| CoordError::Config("no available LLMs".into()))?
            }
        }
    } else {
        req.available_llms
            .first()
            .cloned()
            .ok_or_else(|| CoordError::Config("no available LLMs".into()))?
    };

    // ── Étape 3 : Compressor (optionnel — appliqué si texte long) ─────
    let final_prompt = if let Some(compressor) = &coord.clients.compressor {
        if req.user_text.len() > 500 {
            let _ = coord
                .bus
                .publish(StreamEvent::RobotStarted {
                    swarm_id,
                    robot_id: "compressor".into(),
                    ts: Utc::now(),
                })
                .await;
            let started_node = Instant::now();
            let c_input = CompressorInput {
                text: req.user_text.clone(),
                target_words: 100,
                must_preserve: vec![],
                style: CompressionStyle::Extractive,
            };
            match compressor.invoke(c_input, &ctx).await {
                Ok(out) => {
                    let latency = started_node.elapsed().as_millis() as u64;
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
                    let latency = started_node.elapsed().as_millis() as u64;
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
                    req.user_text.clone()
                }
            }
        } else {
            req.user_text.clone()
        }
    } else {
        req.user_text.clone()
    };

    // ── Étape 4 : appel LLM en streaming ─────────────────────────────
    let llm_call = LlmCall {
        prompt: final_prompt.clone(),
        max_tokens: 1000,
        temperature: 0.7,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("swarm:{swarm_id}:mission_a")),
    };
    let (chunk_tx, mut chunk_rx) =
        tokio::sync::mpsc::channel::<crate::llm::TokenChunk>(512);
    let bus_for_pump = Arc::clone(&coord.bus);
    let provider_for_pump = selected_llm.clone();
    let swarm_for_pump = swarm_id;
    let pump = tokio::spawn(async move {
        while let Some(chunk) = chunk_rx.recv().await {
            let _ = bus_for_pump
                .publish(StreamEvent::LlmTokenStreamed {
                    swarm_id: swarm_for_pump,
                    provider: provider_for_pump.clone(),
                    token: chunk.token,
                    index: chunk.index,
                    ts: Utc::now(),
                })
                .await;
        }
    });
    let llm_resp = coord
        .bridge
        .stream_one(&selected_llm, llm_call, chunk_tx)
        .await?;
    let _ = pump.await;
    let _ = coord
        .bus
        .publish(StreamEvent::LlmFinished {
            swarm_id,
            provider: llm_resp.provider.clone(),
            tokens_generated: llm_resp.tokens_generated,
            cost_cents: llm_resp.cost_cents,
            latency_ms: llm_resp.latency_ms,
            ts: Utc::now(),
        })
        .await;

    // ── Étape 5 : Verifier (si sources) ──────────────────────────────
    let mut verdicts = Vec::new();
    if !req.candidate_sources.is_empty() {
        if let Some(verifier) = &coord.clients.verifier {
            let v_input = VerifierInput {
                claim: llm_resp.text.clone(),
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
            match verifier.invoke(v_input, &ctx).await {
                Ok(out) => {
                    let verdict_str = format!("{:?}", out.verdict).to_uppercase();
                    let severity_str = format!("{:?}", out.severity).to_uppercase();
                    let _ = coord
                        .bus
                        .publish(StreamEvent::VerdictReady {
                            swarm_id,
                            claim: llm_resp.text.clone(),
                            verdict: verdict_str.clone(),
                            severity: severity_str.clone(),
                            best_source_id: out.best_source_id.clone(),
                            similarity: out.similarity,
                            ts: Utc::now(),
                        })
                        .await;
                    verdicts.push(FactCheck {
                        claim: llm_resp.text.clone(),
                        verdict: verdict_str,
                        severity: severity_str,
                        best_source_id: out.best_source_id,
                        similarity: out.similarity,
                    });
                }
                Err(_) => {
                    // Verifier optionnel — on continue sans verdicts.
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

    let response = MissionAResponse {
        final_text: llm_resp.text,
        llm_used: llm_resp.provider,
        cost_cents: llm_resp.cost_cents,
        latency_ms: started.elapsed().as_millis() as u64,
        verdicts,
        audit_chain_hash: audit_hash.clone(),
    };

    let _ = coord
        .bus
        .publish(StreamEvent::SwarmFinished {
            swarm_id,
            mission: MissionTag::MissionA,
            latency_ms: response.latency_ms,
            audit_chain_hash: audit_hash,
            ts: Utc::now(),
        })
        .await;

    Ok(response)
}
