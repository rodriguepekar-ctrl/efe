// SPDX-License-Identifier: UNLICENSED
//! Workflow Debate — 2 LLMs argumentent (PRO/CONTRA), 1 LLM rend le verdict.

use std::sync::Arc;
use std::time::Instant;

use efe_coordinator::{Coordinator, LlmCall};
use uuid::Uuid;

use crate::budget::BudgetTracker;
use crate::canonical::{ArgumentStance, CanonicalArgument};
use crate::error::{MultiLlmError, MultiLlmResult};
use crate::events_ext::{BudgetCharge, Phase3EventExt, Phase3Publisher};
use crate::reformulator_ext::{reformulator_for, SourceDialect};
use crate::request::{DebateArgumentSummary, DebateRequest, DebateResponse};
use crate::DEBATE_MAX_ROUNDS;

/// Exécute le Debate : 2 débatteurs (PRO/CONTRA) sur N rounds, puis verdict.
pub async fn run_debate(
    coord: &Coordinator,
    req: DebateRequest,
) -> MultiLlmResult<DebateResponse> {
    if req.rounds == 0 || req.rounds > DEBATE_MAX_ROUNDS {
        return Err(MultiLlmError::Config(format!(
            "rounds must be 1..={DEBATE_MAX_ROUNDS}, got {}",
            req.rounds
        )));
    }
    // Besoin de 3 LLMs distincts : pro, contra, jury.
    if req.pro_llm == req.contra_llm
        || req.pro_llm == req.jury_llm
        || req.contra_llm == req.jury_llm
    {
        return Err(MultiLlmError::DebateTooFewLlms { n: 2 });
    }

    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let publisher = Phase3Publisher;
    let budget = BudgetTracker::new(req.max_budget_cents);
    // Fil B.0 : reformulateur pluggable (déterministe par défaut).
    let reformulator = reformulator_for(coord);
    let bus = Arc::clone(&coord.bus);

    let mut arguments_per_round: Vec<Vec<DebateArgumentSummary>> = Vec::new();
    let mut last_pro_argument = String::new();
    let mut last_contra_argument = String::new();
    // Fil B.3 : 2 hops estampillés par round (le verdict jury n'est pas
    // reformulé — pas de hop jury, honnête).
    let mut reformulation_hops = Vec::with_capacity(2 * req.rounds as usize);

    for round_idx in 1..=req.rounds {
        publisher
            .publish_debate_round_started(
                &bus,
                swarm_id,
                round_idx,
                &[req.pro_llm.clone(), req.contra_llm.clone()],
            )
            .await;

        // Construit les prompts (round 1 = opening, rounds suivants = rebuttals).
        let (pro_prompt, contra_prompt) = if round_idx == 1 {
            (
                format!(
                    "You are arguing PRO on this topic.  Present a clear thesis with reasoning bullet-points.\n\nTopic: {}",
                    req.user_text
                ),
                format!(
                    "You are arguing CONTRA on this topic.  Present a clear counter-thesis with reasoning bullet-points.\n\nTopic: {}",
                    req.user_text
                ),
            )
        } else {
            (
                format!(
                    "Rebut the following CONTRA argument.  Strengthen the PRO position.\n\n\
                     Topic: {}\n\nOpponent's CONTRA argument: {last_contra_argument}",
                    req.user_text
                ),
                format!(
                    "Rebut the following PRO argument.  Strengthen the CONTRA position.\n\n\
                     Topic: {}\n\nOpponent's PRO argument: {last_pro_argument}",
                    req.user_text
                ),
            )
        };

        // Appels parallèles.
        let pro_call = LlmCall {
            prompt: pro_prompt,
            max_tokens: 600,
            temperature: 0.6,
            deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
            trace_tag: Some(format!("phase3:debate_pro_r{round_idx}:{swarm_id}")),
        };
        let contra_call = LlmCall {
            prompt: contra_prompt,
            max_tokens: 600,
            temperature: 0.6,
            deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
            trace_tag: Some(format!("phase3:debate_contra_r{round_idx}:{swarm_id}")),
        };

        let (pro_res, contra_res) = tokio::join!(
            coord.bridge.call_one(&req.pro_llm, pro_call),
            coord.bridge.call_one(&req.contra_llm, contra_call),
        );

        let pro_resp = pro_res.map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: format!("pro round {round_idx}"),
            reason: e.to_string(),
        })?;
        let contra_resp = contra_res.map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: format!("contra round {round_idx}"),
            reason: e.to_string(),
        })?;

        let _ = budget.try_charge(&req.pro_llm, pro_resp.cost_cents);
        let _ = budget.try_charge(&req.contra_llm, contra_resp.cost_cents);
        publisher
            .publish_budget_charged(
                &bus,
                swarm_id,
                &req.pro_llm,
                BudgetCharge {
                    cents: pro_resp.cost_cents,
                    running_total: budget.spent_cents(),
                    max_budget: budget.max_cost_cents(),
                },
            )
            .await;
        publisher
            .publish_budget_charged(
                &bus,
                swarm_id,
                &req.contra_llm,
                BudgetCharge {
                    cents: contra_resp.cost_cents,
                    running_total: budget.spent_cents(),
                    max_budget: budget.max_cost_cents(),
                },
            )
            .await;

        // Reformule en CanonicalArgument.
        let pro_stance = if round_idx == 1 {
            ArgumentStance::Pro
        } else {
            ArgumentStance::Rebuttal
        };
        let contra_stance = if round_idx == 1 {
            ArgumentStance::Contra
        } else {
            ArgumentStance::Rebuttal
        };
        let pro_ref = reformulator
            .to_argument_value(
                &pro_resp.text,
                pro_stance,
                SourceDialect::from_provider_id(&req.pro_llm),
                // B.4 : le hop SLM est borné au temps de mission RESTANT.
                Some(req.max_total_latency.saturating_sub(started.elapsed())),
            )
            .await?;
        // Fil B.3 : le hop AVANT le from_value (qui consomme `.value`).
        let pro_hop = pro_ref.to_hop(
            &format!("argument:r{round_idx}:PRO"),
            &req.pro_llm,
            "canonical/argument_v1",
        );
        let pro_canon: CanonicalArgument =
            serde_json::from_value(pro_ref.value).map_err(|e| {
                MultiLlmError::ReformulationFailed {
                    from_dialect: SourceDialect::from_provider_id(&req.pro_llm).tag().to_string(),
                    to_schema: "canonical/argument_v1".to_string(),
                    reason: e.to_string(),
                }
            })?;
        let contra_ref = reformulator
            .to_argument_value(
                &contra_resp.text,
                contra_stance,
                SourceDialect::from_provider_id(&req.contra_llm),
                // B.4 : le hop SLM est borné au temps de mission RESTANT.
                Some(req.max_total_latency.saturating_sub(started.elapsed())),
            )
            .await?;
        let contra_hop = contra_ref.to_hop(
            &format!("argument:r{round_idx}:CONTRA"),
            &req.contra_llm,
            "canonical/argument_v1",
        );
        let contra_canon: CanonicalArgument =
            serde_json::from_value(contra_ref.value).map_err(|e| {
                MultiLlmError::ReformulationFailed {
                    from_dialect: SourceDialect::from_provider_id(&req.contra_llm).tag().to_string(),
                    to_schema: "canonical/argument_v1".to_string(),
                    reason: e.to_string(),
                }
            })?;

        // Fil B.0 : provenance du moteur de reformulation (par argument).
        publisher
            .publish_reformulation(&bus, swarm_id, &req.pro_llm, "canonical/argument_v1", pro_hop.engine)
            .await;
        publisher
            .publish_reformulation(&bus, swarm_id, &req.contra_llm, "canonical/argument_v1", contra_hop.engine)
            .await;
        reformulation_hops.push(pro_hop);
        reformulation_hops.push(contra_hop);

        publisher
            .publish_debate_argument(&bus, swarm_id, round_idx, &req.pro_llm, "PRO")
            .await;
        publisher
            .publish_debate_argument(&bus, swarm_id, round_idx, &req.contra_llm, "CONTRA")
            .await;

        last_pro_argument = pro_canon.thesis.clone();
        last_contra_argument = contra_canon.thesis.clone();

        arguments_per_round.push(vec![
            DebateArgumentSummary {
                author: req.pro_llm.clone(),
                stance: format!("{:?}", pro_canon.stance).to_uppercase(),
                thesis: pro_canon.thesis,
            },
            DebateArgumentSummary {
                author: req.contra_llm.clone(),
                stance: format!("{:?}", contra_canon.stance).to_uppercase(),
                thesis: contra_canon.thesis,
            },
        ]);
    }

    // ── Phase JURY : Gemini rend le verdict ──────────────────────────
    publisher
        .publish_debate_round_started(
            &bus,
            swarm_id,
            req.rounds + 1, // round virtuel "jury"
            &[req.jury_llm.clone()],
        )
        .await;
    let jury_prompt = format!(
        "You are a neutral jury.  Below are arguments from a debate.  Rule on the topic objectively, with a clear verdict and justification citing specific arguments.\n\nTopic: {}\n\nPRO arguments: {last_pro_argument}\n\nCONTRA arguments: {last_contra_argument}",
        req.user_text
    );
    let jury_call = LlmCall {
        prompt: jury_prompt,
        max_tokens: 1000,
        temperature: 0.4,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:debate_jury:{swarm_id}")),
    };
    let jury_resp = coord
        .bridge
        .call_one(&req.jury_llm, jury_call)
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "jury".into(),
            reason: e.to_string(),
        })?;
    let _ = budget.try_charge(&req.jury_llm, jury_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.jury_llm,
            BudgetCharge {
                cents: jury_resp.cost_cents,
                running_total: budget.spent_cents(),
                max_budget: budget.max_cost_cents(),
            },
        )
        .await;

    Ok(DebateResponse {
        verdict: jury_resp.text,
        arguments_per_round,
        jury_llm: req.jury_llm,
        total_cost_cents: budget.spent_cents(),
        latency_ms: started.elapsed().as_millis() as u64,
        swarm_id_hex: swarm_id.simple().to_string(),
        reformulation_hops,
    })
}

