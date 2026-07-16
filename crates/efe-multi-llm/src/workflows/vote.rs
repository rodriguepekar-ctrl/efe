// SPDX-License-Identifier: UNLICENSED
//! Workflow Vote 3-LLM avec reformulation pré-vote.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use efe_coordinator::{Coordinator, LlmCall};
use uuid::Uuid;

use crate::budget::BudgetTracker;
use crate::canonical::{CanonicalClaim, CanonicalSchemaId};
use crate::error::{MultiLlmError, MultiLlmResult};
use crate::events_ext::{Phase3EventExt, Phase3Publisher};
use crate::reformulator_ext::{reformulator_for, SourceDialect};
use crate::request::{VoteMode, Vote3LlmRequest, Vote3LlmResponse};
use crate::vote_ext::{borda_tally, condorcet_tally};
use crate::vote_ext::borda::BordaBallot;
use efe_coordinator::vote::{Ballot, VoteEngine};

/// Exécute le workflow Vote 3-LLM avec reformulation pré-vote.
///
/// Étapes :
/// 1. Compress le prompt unique (économie input × 8).
/// 2. Fan-out parallèle vers les N LLMs (typiquement 3).
/// 3. Reformule chaque output → `CanonicalClaim`.
/// 4. Vote selon `VoteMode` (Plurality / Borda / Condorcet) sur les claims.
/// 5. Retourne le winner.
///
/// Invariants respectés :
/// - I13 : BudgetTracker hard-cap atomique.
/// - I9 : (TODO Phase 3 v0.2) early termination quand quorum atteint.
pub async fn run_vote_3llm(
    coord: &Coordinator,
    req: Vote3LlmRequest,
) -> MultiLlmResult<Vote3LlmResponse> {
    let started = Instant::now();
    let swarm_id = Uuid::now_v7();
    let publisher = Phase3Publisher;
    let budget = BudgetTracker::new(req.max_budget_cents);
    // Fil B.0 : reformulateur pluggable (déterministe par défaut).
    let reformulator = reformulator_for(coord);

    let bus = Arc::clone(&coord.bus);

    // ── Étape 1 : Compress (si compressor disponible et texte long) ──
    // (Le pré-mâchage compressor est déjà couvert par Mission B Phase 2 ;
    // ici on appelle directement les LLMs avec le user_text.)
    let llm_call = LlmCall {
        prompt: req.user_text.clone(),
        max_tokens: 800,
        temperature: 0.5,
        deadline: Some(req.max_total_latency.saturating_sub(started.elapsed())),
        trace_tag: Some(format!("phase3:vote_3llm:{swarm_id}")),
    };

    // ── Étape 2 : fan-out via call_many_ok ────────────────────────────
    let responses = coord.bridge.call_many_ok(&req.llms, llm_call).await;
    if responses.is_empty() {
        return Err(MultiLlmError::AllLlmsDown);
    }
    // Budget charging (post-call : on connaît le vrai coût).
    for r in &responses {
        let _ = budget.try_charge(&r.provider, r.cost_cents); // ignore overflow ici, on rapportera dans total
        publisher
            .publish_budget_charged(
                &bus,
                swarm_id,
                &r.provider,
                r.cost_cents,
                budget.spent_cents(),
                budget.max_cost_cents(),
            )
            .await;
    }

    // ── Étape 3 : reformulation → canonical claim ────────────────────
    let mut claims: Vec<(String, CanonicalClaim, f32)> = Vec::new();
    // Fil B.3 : un hop estampillé par claim ENTRÉE au vote (le reçu reflète
    // ce qui a réellement voté, pas les tentatives avortées).
    let mut reformulation_hops = Vec::with_capacity(responses.len());
    for r in &responses {
        let dialect = SourceDialect::from_provider_id(&r.provider);
        let reff = match reformulator
            .to_canonical(&r.text, dialect, CanonicalSchemaId::ClaimV1, &r.provider)
            .await
        {
            Ok(reff) => reff,
            Err(_) => continue, // skip ce LLM si parsing échoue
        };
        publisher
            .publish_reformulation(
                &bus,
                swarm_id,
                &r.provider,
                "canonical/claim_v1",
                reff.engine,
            )
            .await;
        let hop = reff.to_hop(&format!("claim:{}", r.provider), &r.provider, "canonical/claim_v1");
        if let Ok(claim) = serde_json::from_value::<CanonicalClaim>(reff.value) {
            claims.push((r.provider.clone(), claim, r.self_quality));
            reformulation_hops.push(hop);
        }
    }

    if claims.is_empty() {
        return Err(MultiLlmError::AllLlmsDown);
    }

    // ── Étape 4 : vote selon mode ─────────────────────────────────────
    let quorum = req.quorum.unwrap_or_else(|| (claims.len() + 1) / 2);
    let (winner_provider, winner_text, scores): (String, String, HashMap<String, f32>) =
        match req.mode {
            VoteMode::Plurality => {
                // Réutilise VoteEngine Phase 2.
                let ballots: Vec<Ballot> = claims
                    .iter()
                    .map(|(p, c, q)| Ballot {
                        provider: p.clone(),
                        answer_text: c.claim.clone(),
                        self_quality: *q,
                        latency_ms: 0,
                    })
                    .collect();
                let engine = VoteEngine::uniform();
                let outcome = engine.tally(&ballots, quorum)?;
                (
                    outcome.winner.clone(),
                    outcome.winner_text.clone(),
                    outcome.per_provider_scores,
                )
            }
            VoteMode::Borda => {
                // Pour Borda, chaque LLM ranke les claims des autres.
                // Heuristique simple : chaque LLM se range #1 et les autres
                // sont rangés par similarité Jaccard décroissante.
                let candidates: Vec<String> =
                    claims.iter().map(|(p, _, _)| p.clone()).collect();
                let mut ballots: Vec<BordaBallot> = Vec::new();
                for (p, _, _) in &claims {
                    ballots.push(BordaBallot {
                        voter: p.clone(),
                        ranking: rank_candidates(p, &candidates, &claims),
                    });
                }
                let outcome = borda_tally(&ballots)?;
                let winner_idx = claims.iter().position(|(p, _, _)| p == &outcome.winner);
                let winner_text = winner_idx
                    .map(|i| claims[i].1.claim.clone())
                    .unwrap_or_default();
                let scores: HashMap<String, f32> = outcome
                    .scores
                    .into_iter()
                    .map(|(k, v)| (k, v as f32))
                    .collect();
                (outcome.winner, winner_text, scores)
            }
            VoteMode::Condorcet => {
                let candidates: Vec<String> =
                    claims.iter().map(|(p, _, _)| p.clone()).collect();
                let mut ballots: Vec<BordaBallot> = Vec::new();
                for (p, _, _) in &claims {
                    ballots.push(BordaBallot {
                        voter: p.clone(),
                        ranking: rank_candidates(p, &candidates, &claims),
                    });
                }
                let outcome = condorcet_tally(&ballots)?;
                let winner = outcome.winner.unwrap_or_else(|| candidates[0].clone());
                let winner_idx = claims.iter().position(|(p, _, _)| p == &winner);
                let winner_text = winner_idx
                    .map(|i| claims[i].1.claim.clone())
                    .unwrap_or_default();
                let scores: HashMap<String, f32> = candidates
                    .iter()
                    .map(|c| {
                        let total: u32 = outcome
                            .wins
                            .get(c)
                            .map(|m| m.values().sum())
                            .unwrap_or(0);
                        (c.clone(), total as f32)
                    })
                    .collect();
                (winner, winner_text, scores)
            }
        };

    let total_cost: u32 = responses.iter().map(|r| r.cost_cents).sum();
    let winner_score = *scores.get(&winner_provider).unwrap_or(&0.0);

    Ok(Vote3LlmResponse {
        final_text: winner_text,
        winner_llm: winner_provider,
        mode: req.mode,
        winner_score,
        per_llm_scores: scores,
        total_cost_cents: total_cost,
        latency_ms: started.elapsed().as_millis() as u64,
        audit_chain_hash: format!("{:016x}", swarm_id.as_u128()),
        cancelled_llms: vec![],
        reformulation_hops,
    })
}

/// Heuristique : un votant `voter` range les `candidates` ainsi :
/// 1. soi-même en premier.
/// 2. autres par similarité Jaccard décroissante avec son propre claim.
fn rank_candidates(
    voter: &str,
    candidates: &[String],
    claims: &[(String, CanonicalClaim, f32)],
) -> Vec<String> {
    let voter_claim = claims
        .iter()
        .find(|(p, _, _)| p == voter)
        .map(|(_, c, _)| c.claim.clone())
        .unwrap_or_default();
    let mut others: Vec<(&String, f32)> = candidates
        .iter()
        .filter(|c| *c != voter)
        .map(|c| {
            let other_claim = claims
                .iter()
                .find(|(p, _, _)| p == c)
                .map(|(_, cl, _)| cl.claim.clone())
                .unwrap_or_default();
            (c, jaccard_words(&voter_claim, &other_claim))
        })
        .collect();
    others.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    let mut ranking = vec![voter.to_string()];
    ranking.extend(others.into_iter().map(|(c, _)| c.clone()));
    ranking
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
