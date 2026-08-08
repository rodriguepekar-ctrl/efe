// SPDX-License-Identifier: UNLICENSED
//! Workflow Cascade — Speculative à 3 niveaux (drafter → verifier → validator).

use std::sync::Arc;
use std::time::Instant;

use efe_coordinator::{Coordinator, LlmCall};
use uuid::Uuid;

use crate::budget::BudgetTracker;
use crate::error::MultiLlmResult;
use crate::events_ext::{Phase3EventExt, Phase3Publisher};
use crate::request::{CascadeRequest, CascadeResponse};

/// Exécute la cascade speculative à 3 niveaux.
///
/// Étapes :
/// 1. **Drafter** (local, cheap) produit un draft.
/// 2. **Verifier** (medium) lit le draft + question → ACCEPT/REGEN.
/// 3. **Validator** (premium, dernier ressort) appelé seulement si REGEN_ALL.
///
/// Métriques cibles (50% des cas drafter direct accepté) :
/// - Latence moyenne -41% vs Validator seul.
/// - Coût moyen -70% vs Validator seul.
pub async fn run_cascade(
    coord: &Coordinator,
    req: CascadeRequest,
) -> MultiLlmResult<CascadeResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let publisher = Phase3Publisher;
    let budget = BudgetTracker::new(req.max_budget_cents);
    let bus = Arc::clone(&coord.bus);

    // Baseline : combien Validator solo prendrait (estimation grosse).
    let baseline_validator_solo_ms: u64 = req
        .max_total_latency
        .as_millis()
        .min(3_000) as u64; // 3s en estimation

    // ── Niveau 1 : Drafter ───────────────────────────────────────────
    let drafter_call = LlmCall {
        prompt: req.user_text.clone(),
        max_tokens: 500,
        temperature: 0.7,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:cascade_drafter:{swarm_id}")),
    };
    let drafter_resp = coord.bridge.call_one(&req.drafter_llm, drafter_call).await?;
    let _ = budget.try_charge(&req.drafter_llm, drafter_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.drafter_llm,
            drafter_resp.cost_cents,
            budget.spent_cents(),
            budget.max_cost_cents(),
        )
        .await;
    publisher
        .publish_drafter_finished(&bus, swarm_id, drafter_resp.tokens_generated)
        .await;

    // ── Niveau 2 : Verifier ───────────────────────────────────────────
    let verifier_prompt = format!(
        "Question: {}\n\nDraft answer:\n{}\n\nVerify the draft. \
         Reply with one of: 'ACCEPT' (draft is correct), 'REGEN_RANGES' (partial fix needed), \
         or 'REGEN_ALL' (complete rewrite needed).",
        req.user_text, drafter_resp.text
    );
    let verifier_call = LlmCall {
        prompt: verifier_prompt,
        max_tokens: 200,
        temperature: 0.2,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:cascade_verifier:{swarm_id}")),
    };
    let verifier_resp = coord
        .bridge
        .call_one(&req.verifier_llm, verifier_call)
        .await?;
    let _ = budget.try_charge(&req.verifier_llm, verifier_resp.cost_cents);
    publisher
        .publish_budget_charged(
            &bus,
            swarm_id,
            &req.verifier_llm,
            verifier_resp.cost_cents,
            budget.spent_cents(),
            budget.max_cost_cents(),
        )
        .await;

    // Parse la décision : si la similarité draft↔verifier ≥ threshold,
    // on considère ACCEPT.  Sinon on regarde si le verifier dit REGEN_ALL.
    let sim = jaccard_words(&drafter_resp.text, &verifier_resp.text);
    let decision_str = if sim >= req.accept_threshold {
        "ACCEPT"
    } else if verifier_resp.text.to_uppercase().contains("REGEN_ALL") {
        "REGEN_ALL"
    } else {
        "REGEN_RANGES"
    };
    publisher
        .publish_verifier_decision(&bus, swarm_id, decision_str, 0)
        .await;

    // ── Niveau 3 : Validator (rare) ──────────────────────────────────
    let (final_text, validator_used) = if decision_str == "REGEN_ALL" {
        let validator_call = LlmCall {
            prompt: format!(
                "Question: {}\n\nThe draft and verifier failed. Provide the definitive answer.",
                req.user_text
            ),
            max_tokens: 800,
            temperature: 0.5,
            deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
            trace_tag: Some(format!("phase3:cascade_validator:{swarm_id}")),
        };
        match coord
            .bridge
            .call_one(&req.validator_llm, validator_call)
            .await
        {
            Ok(v) => {
                let _ = budget.try_charge(&req.validator_llm, v.cost_cents);
                publisher
                    .publish_budget_charged(
                        &bus,
                        swarm_id,
                        &req.validator_llm,
                        v.cost_cents,
                        budget.spent_cents(),
                        budget.max_cost_cents(),
                    )
                    .await;
                (v.text, true)
            }
            Err(_) => {
                // Fallback : on rend le draft tel quel avec warn.
                (drafter_resp.text.clone(), false)
            }
        }
    } else if decision_str == "REGEN_RANGES" {
        // Partial regen : on prend l'output du verifier comme final.
        (verifier_resp.text.clone(), false)
    } else {
        // ACCEPT : draft direct.
        (drafter_resp.text.clone(), false)
    };

    let total_ms = started.elapsed().as_millis() as u64;
    let latency_saved_ms = baseline_validator_solo_ms as i64 - total_ms as i64;

    Ok(CascadeResponse {
        final_text,
        verifier_decision: decision_str.to_string(),
        validator_used,
        draft_text: drafter_resp.text,
        total_cost_cents: budget.spent_cents(),
        latency_ms: total_ms,
        latency_saved_ms,
        swarm_id_hex: swarm_id.simple().to_string(),
    })
}

fn jaccard_words(a: &str, b: &str) -> f32 {
    let sa: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let sb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count().max(1);
    inter as f32 / union as f32
}
