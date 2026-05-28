// SPDX-License-Identifier: UNLICENSED
//! Mission A : pré-mâcher pour 1 LLM gros.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use efe_nano_app_coordinator::clients::router::RouterClient;
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
use crate::failure::FailureMode;
use crate::llm::LlmCall;
use crate::metrics::ChewingMetrics;
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
//
// `field_reassign_with_default` : pattern voulu — on accumule incrémentalement
// les champs de `ChewingMetrics` au fil des phases de la mission. La syntaxe
// struct littéral n'est pas applicable car les valeurs viennent de différents
// blocs `match`/branches.
#[allow(clippy::field_reassign_with_default)]
pub async fn run_mission_a(
    coord: &Coordinator,
    req: MissionARequest,
) -> CoordResult<MissionAResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let mut audit_entries: Vec<Uuid> = vec![];

    // ── Sprint S4 R2 : ChewingMetrics tracking ──────────────────────────
    // Initialisation avec les tokens d'entrée bruts (avant pré-mâchage).
    // Tous les autres champs seront remplis incrémentalement à chaque étape.
    let mut chewing_metrics = ChewingMetrics::default();
    chewing_metrics.input_tokens_raw = ChewingMetrics::estimate_tokens(&req.user_text);

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
        chewing_metrics.analyzer_latency_ms = latency;
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
    //
    // S6 R2 : si la requête fournit une `task_class_scoring_table`, on
    // construit un `RouterClient` tuné pour la `task_class` à la volée
    // (poids Pareto = ceux du `weights_per_task` de la table). Sinon on
    // utilise le router instancié au démarrage du Coordinator (poids
    // uniformes 1.0/1.0/1.0).
    //
    // `RouterClient: Copy` (3 × f32 = 12 bytes) → le binding owned est
    // trivialement déplaçable, pas de coût de clone Arc.
    let router_for_call: Option<RouterClient> =
        if let Some(table) = req.task_class_scoring_table.as_ref() {
            Some(RouterClient::for_task_class(table.as_ref(), req.task_class))
        } else {
            coord.clients.router.as_deref().copied()
        };
    let selected_llm = if let Some(router) = router_for_call.as_ref() {
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
                chewing_metrics.router_latency_ms = latency;
                chewing_metrics.router_reason = out.reason.clone();
                chewing_metrics.router_estimated_cost_cents = out.estimated_cost_cents;
                // R3.S1 : si le router R3 a peuplé `estimated_cost_microcents`,
                // on l'utilise ; sinon fallback `cents × 1000`.
                chewing_metrics.router_estimated_cost_microcents = if out.estimated_cost_microcents > 0 {
                    out.estimated_cost_microcents
                } else {
                    (out.estimated_cost_cents as u64) * 1000
                };
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
                chewing_metrics.router_latency_ms = latency;
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
    // Le LLM finalement choisi (par router ou fallback) est tracé dans les métriques.
    chewing_metrics.selected_llm = selected_llm.clone();

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
                // S5 R2 patch (symétrie avec mission_b.rs) : target_words=100
                // était trop lâche, le modèle produisait des résumés ≈ même
                // taille que l'input. 30 mots ≈ 30 tokens → savings ≈ 100,
                // ratio ≈ 0.22. S4 avait passé par chance non-déterministe.
                target_words: 30,
                must_preserve: vec![],
                style: CompressionStyle::Extractive,
            };
            match compressor.invoke(c_input, &ctx).await {
                Ok(out) => {
                    let latency = started_node.elapsed().as_millis() as u64;
                    chewing_metrics.compressor_latency_ms = latency;
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
                    chewing_metrics.compressor_latency_ms = latency;
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

    // S4 R2 : capture des tokens compressés (le LLM va recevoir final_prompt).
    chewing_metrics.input_tokens_compressed = ChewingMetrics::estimate_tokens(&final_prompt);
    chewing_metrics.recompute_savings();

    // ── Étape 4 : appel LLM avec fallback chain S8 ─────────────────────
    //
    // S8 R2 : si le `selected_llm` fail (HTTP 429/5xx/transport après retry
    // épuisé), on bascule sur les autres LLMs disponibles dans
    // `req.available_llms`. Le bus event `LLM_FALLBACK_TRIGGERED` trace la
    // bascule. La mission est préservée si AU MOINS UN candidat répond.
    //
    // Ordre des candidats : selected_llm en premier, puis les autres LLMs
    // disponibles dans l'ordre fourni (= ordre de `req.available_llms`).
    let candidates: Vec<String> = std::iter::once(selected_llm.clone())
        .chain(
            req.available_llms
                .iter()
                .filter(|l| **l != selected_llm)
                .cloned(),
        )
        .collect();

    let mut llm_resp_opt: Option<crate::llm::LlmResponse> = None;
    let mut last_err: Option<CoordError> = None;
    let mut fallback_chain_used: Option<String> = None;
    let mut failure_mode = FailureMode::None;
    let mut failed_components: Vec<String> = Vec::new();

    for (i, candidate) in candidates.iter().enumerate() {
        let llm_call = LlmCall {
            prompt: final_prompt.clone(),
            max_tokens: 1000,
            temperature: 0.7,
            deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
            trace_tag: Some(format!("swarm:{swarm_id}:mission_a:try{i}")),
        };
        let (chunk_tx, mut chunk_rx) =
            tokio::sync::mpsc::channel::<crate::llm::TokenChunk>(512);
        let bus_for_pump = Arc::clone(&coord.bus);
        let provider_for_pump = candidate.clone();
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
        let attempt = coord
            .bridge
            .stream_one(candidate, llm_call, chunk_tx)
            .await;
        let _ = pump.await;

        match attempt {
            Ok(r) => {
                if i > 0 {
                    fallback_chain_used = Some(candidate.clone());
                    failure_mode = FailureMode::LlmFallbackTriggered {
                        from: selected_llm.clone(),
                        to: candidate.clone(),
                    };
                    let reason = last_err
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let _ = coord
                        .bus
                        .publish(StreamEvent::LlmFallbackTriggered {
                            swarm_id,
                            from: selected_llm.clone(),
                            to: candidate.clone(),
                            reason,
                            ts: Utc::now(),
                        })
                        .await;
                }
                llm_resp_opt = Some(r);
                break;
            }
            Err(e) => {
                failed_components.push(format!("llm:{candidate}"));
                last_err = Some(e);
                // Continue avec le prochain candidat.
            }
        }
    }

    let llm_resp = llm_resp_opt.ok_or_else(|| {
        last_err.unwrap_or_else(|| CoordError::LlmProvider {
            provider: "all candidates exhausted".into(),
            reason: format!(
                "all {} LLM candidates failed in run_mission_a",
                candidates.len()
            ),
        })
    })?;
    // S4 R2 : capture metrics LLM (tokens + latence venant du provider).
    chewing_metrics.llm_latency_ms = llm_resp.latency_ms;
    chewing_metrics.llm_tokens_generated = llm_resp.tokens_generated;
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
            // S4 R2 : instrumenter la latence du verifier (manquait dans le code original).
            let verifier_started = Instant::now();
            match verifier.invoke(v_input, &ctx).await {
                Ok(out) => {
                    chewing_metrics.verifier_latency_ms =
                        verifier_started.elapsed().as_millis() as u64;
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
                    chewing_metrics.verifier_latency_ms =
                        verifier_started.elapsed().as_millis() as u64;
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

    // ── Sprint S4 R2 : finalisation des ChewingMetrics ────────────────
    // À ce point, toutes les latences par phase sont déjà tracées. On calcule
    // les agrégats (somme SLM + total pipeline) et on émet l'event PRE_CHEWING_METRICS
    // avant SWARM_FINISHED pour permettre aux consommateurs (UI / tests) de
    // voir les métriques en parallèle de la fin de mission.
    chewing_metrics.total_pipeline_latency_ms = started.elapsed().as_millis() as u64;
    chewing_metrics.recompute_total_slm_latency();

    // S8 R2 : wire les champs de failure tracking dans chewing_metrics.
    chewing_metrics.failure_mode = failure_mode.clone();
    chewing_metrics.degradations_count = failure_mode.count_degradations();
    chewing_metrics.failed_components = failed_components;
    // selected_llm reflète le LLM réellement utilisé (post-fallback).
    chewing_metrics.selected_llm = llm_resp.provider.clone();

    let _ = coord
        .bus
        .publish(StreamEvent::PreChewingMetrics {
            swarm_id,
            metrics: chewing_metrics.clone(),
            ts: Utc::now(),
        })
        .await;

    // R3.S1 : propagation explicite cost_microcents (resolved fallback si absent).
    let cost_mc = llm_resp.cost_microcents_resolved();
    let response = MissionAResponse {
        final_text: llm_resp.text,
        llm_used: llm_resp.provider,
        cost_cents: llm_resp.cost_cents,
        cost_microcents: cost_mc,
        latency_ms: started.elapsed().as_millis() as u64,
        verdicts,
        audit_chain_hash: audit_hash.clone(),
        chewing_metrics: Some(chewing_metrics),
        // S8 R2 : fallback tracing
        fallback_chain_used,
        failure_mode,
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
