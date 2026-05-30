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
// R3.S2 (fix L2) : helpers SLM circuit guards.
use crate::workflows::slm_guard::{
    slm_guard_check, slm_guard_record_failure, slm_guard_record_success, SlmGuardOutcome,
};
use crate::metering::{MissionMetricGuard, MissionRecord, MissionType};
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
/// Exécute la Mission A (génère un `swarm_id` frais).
///
/// **Wrapper backward-compat** : délègue à [`run_mission_a_with_swarm_id`] avec
/// `Uuid::now_v7()`. Tous les callers historiques restent inchangés.
pub async fn run_mission_a(
    coord: &Coordinator,
    req: MissionARequest,
) -> CoordResult<MissionAResponse> {
    run_mission_a_with_swarm_id(coord, req, Uuid::now_v7()).await
}

/// **P4.S1** — Mission A avec `swarm_id` injecté par le caller.
///
/// Identique à [`run_mission_a`] mais le `swarm_id` est fourni. Permet au bridge
/// SSE ([`crate::Coordinator::run_mission_a_streamed`]) de s'abonner au bus
/// **avant** que la mission publie `SwarmStarted` — abonnement sans course,
/// zéro event manqué.
#[allow(clippy::field_reassign_with_default)]
pub async fn run_mission_a_with_swarm_id(
    coord: &Coordinator,
    mut req: MissionARequest,
    swarm_id: Uuid,
) -> CoordResult<MissionAResponse> {
    let started = Instant::now();
    let mut audit_entries: Vec<Uuid> = vec![];

    // R3.S7 : garde télémétrie. Armée → un retour `?` (erreur) enregistre
    // `outcome="error"` via Drop ; `record_success` (avant Ok) désarme +
    // enregistre les métriques détaillées. Inerte si `coord.metrics` est None.
    let mut metric_guard = MissionMetricGuard::arm(coord.metrics.clone(), MissionType::A);

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

    // ── R3.S5 : résolution multi-tenant + enforcement des politiques ──────
    //
    // Résout le tenant effectif (requête > default global) et sa policy. Tous
    // les leviers (allow-list, scoring, budget) sont appliqués EN AMONT du
    // pipeline, en mutant `req` — ainsi le code aval (router L195, fallback
    // L269) consomme automatiquement les valeurs enforce-ées, sans churn.
    let (tenant_id, policy) =
        crate::multi_tenant::resolve(&coord.policies, req.tenant_id, coord.config.default_tenant);
    let has_policy = policy.is_some();

    // ── R3.S8 : A/B test de tables ────────────────────────────────────────
    //
    // Précédence : `request > A/B > tenant (R3.S5) > global`. L'A/B ne
    // s'applique QUE si la requête n'épingle pas de table (sinon le caller a
    // choisi). Il remplit alors `req.task_class_scoring_table` AVANT l'injection
    // de la table tenant (plus bas) — qui ne s'activera donc pas. `fixture =
    // None` en live (pas d'identité fixture ; cf. architecture §4.4).
    if req.task_class_scoring_table.is_none() {
        if let Some(ab) = &coord.ab_test {
            if let Some(arm) = ab.assign_arm(swarm_id, tenant_id, None) {
                req.task_class_scoring_table = Some(ab.table_for(arm));
                chewing_metrics.ab_arm = Some(arm);
                // R3.S8 : trace la version de table du bras (audit + rapport A/B).
                chewing_metrics.ab_table_version = Some(ab.version_of(arm));
            }
        }
    }

    // (a) Allow-list : `available_llms ∩ policy.allowed_llms` (jamais union).
    //     Une intersection vide ⇒ fail-closed (pas de fuite vers un LLM hors
    //     plan, pas de fallback silencieux).
    let mut tenant_llms_filtered: Option<usize> = None;
    if let Some(p) = policy {
        let before = req.available_llms.len();
        req.available_llms = p.filter_llms(&req.available_llms);
        tenant_llms_filtered = Some(before - req.available_llms.len());
        if req.available_llms.is_empty() {
            return Err(CoordError::Config(
                "no LLM permitted for tenant (allow-list intersection empty)".into(),
            ));
        }
        // (b) Scoring table : la requête prime (champ non-sécuritaire) ; sinon
        //     on injecte la table du tenant. Le router (L195) la consomme tel
        //     quel via `req.task_class_scoring_table`.
        if req.task_class_scoring_table.is_none() {
            req.task_class_scoring_table = p.scoring_table.clone();
        }
        // (c) Plafond de coût : resserre le budget µ¢ AVANT le filtre router
        //     (router.rs:118 filtre dessus). `min(req, policy)` = plus strict.
        let capped = p.cap_microcents(req.budget.max_cost_microcents_resolved());
        req.budget.max_cost_microcents = capped;
    }
    chewing_metrics.tenant_id = Some(tenant_id);
    chewing_metrics.tenant_llms_filtered = tenant_llms_filtered;

    // Construit CallContext avec le tenant RÉSOLU (fin de la fiction L5-a :
    // l'audit chain trace enfin le vrai tenant, plus `default_tenant` global).
    let audit = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::with_swarm(tenant_id, swarm_id, audit.clone());

    // ── Étape 1 : Analyzer (R3.S2 fix L2 : guard SLM circuit) ────────
    if let Some(analyzer) = &coord.clients.analyzer {
        let circuit = coord.slm_circuit_for(tenant_id, "analyzer", has_policy);
        match slm_guard_check(circuit, "analyzer", &mut chewing_metrics) {
            SlmGuardOutcome::Skip => {
                // R3.S2 : circuit Open → skip analyzer immédiat (<1ms).
                // chewing_metrics.failure_mode/failed_components déjà tracés.
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
                // chewing_metrics.analyzer_latency_ms reste 0 (signal "skipped").
            }
            SlmGuardOutcome::Proceed => {
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
                        // R3.S2 : succès → reset compteur du circuit.
                        slm_guard_record_success(circuit);
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
                        // R3.S2 : échec → record_failure, potentiellement
                        // ouvre circuit + émet SlmCircuitOpened event.
                        // Auto-trace FailureMode + failed_components.
                        let _opened = slm_guard_record_failure(
                            circuit,
                            "analyzer",
                            &mut chewing_metrics,
                            &coord.bus,
                            swarm_id,
                        )
                        .await;
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

    // ── Étape 3 : Compressor (R3.S2 fix L2 : guard SLM circuit) ─────
    let final_prompt = if let Some(compressor) = &coord.clients.compressor {
        if req.user_text.len() > 500 {
            let circuit = coord.slm_circuit_for(tenant_id, "compressor", has_policy);
            match slm_guard_check(circuit, "compressor", &mut chewing_metrics) {
                SlmGuardOutcome::Skip => {
                    // R3.S2 : circuit Open → skip compressor, utilise raw prompt.
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
                            // R3.S2 : succès → reset circuit.
                            slm_guard_record_success(circuit);
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
                            // R3.S2 : échec → record_failure + auto-emit event si transition.
                            let _opened = slm_guard_record_failure(
                                circuit,
                                "compressor",
                                &mut chewing_metrics,
                                &coord.bus,
                                swarm_id,
                            )
                            .await;
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

    // ── Étape 5 : Verifier (R3.S2 fix L2 : guard SLM circuit + si sources) ──
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
                    // S4 R2 : instrumenter la latence du verifier.
                    let verifier_started = Instant::now();
                    match verifier.invoke(v_input, &ctx).await {
                        Ok(out) => {
                            // R3.S2 : succès → reset circuit.
                            slm_guard_record_success(circuit);
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
                            // R3.S2 : échec → record_failure + auto-emit event.
                            let _opened = slm_guard_record_failure(
                                circuit,
                                "verifier",
                                &mut chewing_metrics,
                                &coord.bus,
                                swarm_id,
                            )
                            .await;
                            chewing_metrics.verifier_latency_ms =
                                verifier_started.elapsed().as_millis() as u64;
                            // Verifier optionnel — on continue sans verdicts.
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

    // ── Sprint S4 R2 : finalisation des ChewingMetrics ────────────────
    // À ce point, toutes les latences par phase sont déjà tracées. On calcule
    // les agrégats (somme SLM + total pipeline) et on émet l'event PRE_CHEWING_METRICS
    // avant SWARM_FINISHED pour permettre aux consommateurs (UI / tests) de
    // voir les métriques en parallèle de la fin de mission.
    chewing_metrics.total_pipeline_latency_ms = started.elapsed().as_millis() as u64;
    chewing_metrics.recompute_total_slm_latency();

    // S8 R2 + R3.S2 : wire les champs de failure tracking dans chewing_metrics.
    //
    // R3.S2 chirurgicale : les guards SLM ont déjà tracé failure_mode +
    // failed_components dans chewing_metrics au fil de l'exécution. Le
    // tracking S8 LLM fallback ci-dessous compose AVEC les traces SLM via
    // `combine_failure_modes` (pas d'écrasement).
    let combined_failure_mode = crate::failure::combine_failure_modes(
        std::mem::take(&mut chewing_metrics.failure_mode),
        failure_mode.clone(),
    );
    chewing_metrics.failure_mode = combined_failure_mode;
    chewing_metrics.degradations_count =
        chewing_metrics.failure_mode.count_degradations();
    // failed_components : merge (les SLM ont push, les LLM fallback ajoutent).
    for c in failed_components {
        if !chewing_metrics.failed_components.contains(&c) {
            chewing_metrics.failed_components.push(c);
        }
    }
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
    // R3.S2 : la `failure_mode` finale = combinaison SLM circuits (tracés
    // dans chewing_metrics.failure_mode) + LLM fallback S8 (local `failure_mode`).
    let final_failure_mode = chewing_metrics.failure_mode.clone();
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
        // R3.S2 : failure_mode combiné (SLM + LLM fallback).
        failure_mode: final_failure_mode,
    };
    // R3.S2 : variable locale `failure_mode` n'est plus utilisée dans la
    // response (la valeur est désormais sur chewing_metrics + duplicated).
    // On garde le binding S8 pour minimiser le diff, mais il est mort.
    let _ = failure_mode;

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

    // R3.S7 : enregistre la mission réussie (désarme le guard).
    metric_guard.record_success(&MissionRecord::from_mission_a(&response));
    Ok(response)
}
