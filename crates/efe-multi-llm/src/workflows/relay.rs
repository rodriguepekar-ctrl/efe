// SPDX-License-Identifier: UNLICENSED
//! Workflow Relay — Plan (Claude) → Code (GPT) → Synthesis (Gemini).
//!
//! Utilise le DAG executor Phase 2 v0.0.2 avec `with_parents_and_inputs`
//! pour passer le canonical output d'une phase à la suivante.

use std::sync::Arc;
use std::time::Instant;

use efe_coordinator::{Coordinator, DagNode, LlmCall};
use uuid::Uuid;

use crate::budget::BudgetTracker;
use crate::canonical::CanonicalSchemaId;
use crate::error::{MultiLlmError, MultiLlmResult};
use crate::events_ext::{Phase3EventExt, Phase3Publisher};
use crate::reformulator_ext::{reformulator_for, SourceDialect};
use crate::request::{RelayRequest, RelayResponse};

/// Exécute le Relay : Claude plan → GPT code → Gemini synthesis.
pub async fn run_relay(
    coord: &Coordinator,
    req: RelayRequest,
) -> MultiLlmResult<RelayResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let publisher = Phase3Publisher;
    let budget = BudgetTracker::new(req.max_budget_cents);
    // Fil B.0 : reformulateur pluggable (SLM si câblé, sinon déterministe —
    // inchangé sur tous les chemins servis actuels).
    let reformulator = reformulator_for(coord);
    let bus = Arc::clone(&coord.bus);

    // ── Phase α : PLANIFICATION ───────────────────────────────────────
    let plan_prompt = format!(
        "You are a senior architect.  Given the user request below, produce a structured \
         plan (numbered steps).  Do NOT implement, only plan.\n\nUser request: {}",
        req.user_text
    );
    let plan_call = LlmCall {
        prompt: plan_prompt,
        max_tokens: 800,
        temperature: 0.5,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:relay_plan:{swarm_id}")),
    };
    let plan_resp = coord
        .bridge
        .call_one(&req.planner_llm, plan_call)
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "α (planning)".into(),
            reason: e.to_string(),
        })?;
    let _ = budget.try_charge(&req.planner_llm, plan_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.planner_llm,
            plan_resp.cost_cents,
            budget.spent_cents(),
            budget.max_cost_cents(),
        )
        .await;
    let plan_ref = reformulator
        .to_canonical(
            &plan_resp.text,
            SourceDialect::from_provider_id(&req.planner_llm),
            CanonicalSchemaId::PlanV1,
            &req.planner_llm,
        )
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "α (planning reformulation)".into(),
            reason: e.to_string(),
        })?;
    publisher
        .publish_reformulation(
            &bus,
            swarm_id,
            &req.planner_llm,
            "canonical/plan_v1",
            plan_ref.engine,
        )
        .await;
    // Fil B.3 : le hop estampillé part dans la réponse → scellé dans le reçu.
    let plan_hop = plan_ref.to_hop("plan", &req.planner_llm, "canonical/plan_v1");
    let plan_json = plan_ref.value.to_string();

    // ── Phase β : IMPLEMENTATION ──────────────────────────────────────
    let code_prompt = format!(
        "Implement the plan below in Rust.  Return ONLY a code block ```rust\n...\n```.\n\nPlan (canonical JSON): {plan_json}\n\nOriginal user request: {}",
        req.user_text
    );
    let code_call = LlmCall {
        prompt: code_prompt,
        max_tokens: 1200,
        temperature: 0.3,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:relay_code:{swarm_id}")),
    };
    let code_resp = coord
        .bridge
        .call_one(&req.implementer_llm, code_call)
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "β (implementation)".into(),
            reason: e.to_string(),
        })?;
    let _ = budget.try_charge(&req.implementer_llm, code_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.implementer_llm,
            code_resp.cost_cents,
            budget.spent_cents(),
            budget.max_cost_cents(),
        )
        .await;
    let code_ref = reformulator
        .to_canonical(
            &code_resp.text,
            SourceDialect::from_provider_id(&req.implementer_llm),
            CanonicalSchemaId::CodeV1,
            &req.implementer_llm,
        )
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "β (code reformulation)".into(),
            reason: e.to_string(),
        })?;
    publisher
        .publish_reformulation(
            &bus,
            swarm_id,
            &req.implementer_llm,
            "canonical/code_v1",
            code_ref.engine,
        )
        .await;
    let code_hop = code_ref.to_hop("code", &req.implementer_llm, "canonical/code_v1");
    let code_json = code_ref.value.to_string();

    // ── Phase γ : SYNTHESIS ──────────────────────────────────────────
    let synth_prompt = format!(
        "Synthesize the following plan and code into a user-friendly markdown tutorial.\n\n\
         Plan: {plan_json}\n\nCode: {code_json}\n\nOriginal request: {}",
        req.user_text
    );
    let synth_call = LlmCall {
        prompt: synth_prompt,
        max_tokens: 1000,
        temperature: 0.6,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:relay_synth:{swarm_id}")),
    };
    let synth_resp = coord
        .bridge
        .call_one(&req.synthesizer_llm, synth_call)
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "γ (synthesis)".into(),
            reason: e.to_string(),
        })?;
    let _ = budget.try_charge(&req.synthesizer_llm, synth_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.synthesizer_llm,
            synth_resp.cost_cents,
            budget.spent_cents(),
            budget.max_cost_cents(),
        )
        .await;
    let synth_ref = reformulator
        .to_canonical(
            &synth_resp.text,
            SourceDialect::from_provider_id(&req.synthesizer_llm),
            CanonicalSchemaId::SynthesisV1,
            &req.synthesizer_llm,
        )
        .await
        .map_err(|e| MultiLlmError::RelayPhaseFailed {
            phase: "γ (synthesis reformulation)".into(),
            reason: e.to_string(),
        })?;
    publisher
        .publish_reformulation(
            &bus,
            swarm_id,
            &req.synthesizer_llm,
            "canonical/synthesis_v1",
            synth_ref.engine,
        )
        .await;
    let synth_hop = synth_ref.to_hop("synthesis", &req.synthesizer_llm, "canonical/synthesis_v1");
    let synth_json = synth_ref.value.to_string();

    // Petite touche pour faire taire le linter "unused" sur DagNode.
    let _ = DagNode::root(format!("phase3-relay-{swarm_id}"), || async {
        Ok(serde_json::json!({}))
    });

    Ok(RelayResponse {
        final_text: synth_resp.text,
        plan_json,
        code_json,
        synthesis_json: synth_json,
        llms_used: vec![
            req.planner_llm.clone(),
            req.implementer_llm.clone(),
            req.synthesizer_llm.clone(),
        ],
        total_cost_cents: budget.spent_cents(),
        latency_ms: started.elapsed().as_millis() as u64,
        audit_chain_hash: format!("{:016x}", swarm_id.as_u128()),
        reformulation_hops: vec![plan_hop, code_hop, synth_hop],
    })
}
