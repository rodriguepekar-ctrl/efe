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
use crate::request::{SpeculativeRequest, SpeculativeResponse};
use crate::speculate::Speculator;

/// Exécute la Mission speculative.
///
/// Le drafter est trouvé dans le registre sous l'ID "drafter"
/// (à enregistrer au boot — pratique commune : MockLlmProvider
/// "drafter" pour tests, un SLM local en prod).
///
/// Le `verifier_llm` du request est le gros LLM (Claude/GPT/...)
/// déjà enregistré dans le registre.
pub async fn run_speculative(
    coord: &Coordinator,
    req: SpeculativeRequest,
) -> CoordResult<SpeculativeResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();

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

    let drafter = coord.bridge.registry().get("drafter").ok_or_else(|| {
        CoordError::LlmProvider {
            provider: "drafter".into(),
            reason: "drafter provider not registered".into(),
        }
    })?;
    let verifier = coord.bridge.registry().get(&req.verifier_llm).ok_or_else(|| {
        CoordError::LlmProvider {
            provider: req.verifier_llm.clone(),
            reason: "verifier provider not registered".into(),
        }
    })?;

    let speculator = Arc::new(
        Speculator::new(drafter, verifier, 0.7).with_bus(Arc::clone(&coord.bus), swarm_id),
    );
    let llm_call = LlmCall {
        prompt: req.user_text.clone(),
        max_tokens: req.max_draft_tokens,
        temperature: 0.7,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("swarm:{swarm_id}:speculative")),
    };

    let outcome = speculator.run(llm_call).await?;

    let _ = coord
        .bus
        .publish(StreamEvent::SpeculationResolved {
            swarm_id,
            tokens_accepted: outcome.tokens_accepted,
            tokens_regenerated: outcome.tokens_regenerated,
            latency_saved_ms: outcome.latency_saved_ms,
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

    Ok(resp)
}
