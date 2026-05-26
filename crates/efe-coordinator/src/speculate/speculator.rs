// SPDX-License-Identifier: UNLICENSED
//! Implémentation `Speculator`.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::error::CoordResult;
use crate::events::{EventBus, StreamEvent};
use crate::llm::{LlmCall, LlmProvider, LlmResponse, TokenChunk};

/// Token spéculatif émis par le drafter pendant la frappe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftToken {
    /// Texte du token (souvent 1-4 chars).
    pub token: String,
    /// Position dans le draft.
    pub index: u32,
}

/// Résultat d'une spéculation complète.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculationOutcome {
    /// Réponse finale (validée par le verifier).
    pub final_text: String,
    /// Réponse qu'avait proposée le drafter.
    pub draft_text: String,
    /// Le draft a-t-il été accepté tel quel ?
    pub draft_accepted: bool,
    /// Tokens du draft acceptés.
    pub tokens_accepted: u32,
    /// Tokens regénérés par le verifier.
    pub tokens_regenerated: u32,
    /// Latence gagnée vs baseline (sans spéculation) — ms.
    pub latency_saved_ms: i64,
    /// Latence end-to-end ms.
    pub latency_ms: u64,
    /// Verifier ID.
    pub verifier_id: String,
}

/// Speculator : orchestre drafter + verifier.
pub struct Speculator {
    drafter: Arc<dyn LlmProvider>,
    verifier: Arc<dyn LlmProvider>,
    /// Seuil de similarité au-dessus duquel le draft est accepté tel quel.
    accept_threshold: f32,
    /// Bus optionnel pour émettre `StreamEvent::DraftToken` pendant la frappe.
    bus: Option<Arc<dyn EventBus>>,
    /// `swarm_id` à attacher aux évènements (None → pas d'émission).
    swarm_id: Option<uuid::Uuid>,
}

impl Speculator {
    /// Construit un Speculator (sans bus = aucun évènement émis).
    #[must_use]
    pub fn new(
        drafter: Arc<dyn LlmProvider>,
        verifier: Arc<dyn LlmProvider>,
        accept_threshold: f32,
    ) -> Self {
        Self {
            drafter,
            verifier,
            accept_threshold: accept_threshold.clamp(0.0, 1.0),
            bus: None,
            swarm_id: None,
        }
    }

    /// Attache un bus + swarm_id : chaque token drafté sera émis en
    /// `StreamEvent::DraftToken { swarm_id, token, index, ts }`.
    #[must_use]
    pub fn with_bus(mut self, bus: Arc<dyn EventBus>, swarm_id: uuid::Uuid) -> Self {
        self.bus = Some(bus);
        self.swarm_id = Some(swarm_id);
        self
    }

    /// Drafter ID (re-export pour audit).
    #[must_use]
    pub fn drafter_id(&self) -> String {
        self.drafter.id()
    }

    /// Verifier ID.
    #[must_use]
    pub fn verifier_id(&self) -> String {
        self.verifier.id()
    }

    /// Lance le draft (sur tokio task) + attend le verifier, arbitre.
    ///
    /// Si un bus + swarm_id sont attachés (via [`with_bus`]), chaque
    /// token drafté est émis en temps réel comme `StreamEvent::DraftToken`.
    pub async fn run(&self, req: LlmCall) -> CoordResult<SpeculationOutcome> {
        let started = Instant::now();
        let drafter = Arc::clone(&self.drafter);
        let verifier = Arc::clone(&self.verifier);
        let req_d = req.clone();
        let req_v = req;

        let (tx, rx) = oneshot::channel::<CoordResult<LlmResponse>>();

        // Spawn drafter avec streaming si bus attaché.
        let drafter_started = Instant::now();
        let bus_opt = self.bus.clone();
        let swarm_opt = self.swarm_id;
        let h_draft = tokio::spawn(async move {
            let r = if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
                let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<TokenChunk>(256);
                let bus_c = bus.clone();
                let pump = tokio::spawn(async move {
                    while let Some(chunk) = chunk_rx.recv().await {
                        let _ = bus_c
                            .publish(StreamEvent::DraftToken {
                                swarm_id: swarm,
                                token: chunk.token,
                                index: chunk.index as u32,
                                ts: chrono::Utc::now(),
                            })
                            .await;
                    }
                });
                let r = drafter.stream(req_d, chunk_tx).await;
                let _ = pump.await;
                r
            } else {
                drafter.call(req_d).await
            };
            let _ = tx.send(r);
        });

        // Lance verifier en parallèle.
        let verifier_started = Instant::now();
        let verify_res = verifier.call(req_v).await?;
        let verifier_latency = verifier_started.elapsed().as_millis() as i64;

        // Récupère draft (peut avoir fini avant ou après).
        let draft_res = rx.await.unwrap_or_else(|_| {
            // Le drafter a panic ; on rend un draft vide.
            Ok(LlmResponse {
                text: String::new(),
                tokens_generated: 0,
                cost_cents: 0,
                latency_ms: drafter_started.elapsed().as_millis() as u64,
                provider: self.drafter.id(),
                self_quality: 0.0,
            })
        });
        let _ = h_draft.await;

        let draft = draft_res?;
        let similarity = jaccard_words(&draft.text, &verify_res.text);

        let accept = similarity >= self.accept_threshold;
        let (final_text, tokens_accepted, tokens_regenerated, draft_accepted) = if accept {
            (
                draft.text.clone(),
                draft.tokens_generated,
                0,
                true,
            )
        } else {
            (
                verify_res.text.clone(),
                tokens_accepted_count(&draft.text, &verify_res.text),
                verify_res.tokens_generated
                    .saturating_sub(tokens_accepted_count(&draft.text, &verify_res.text)),
                false,
            )
        };

        let total_latency_ms = started.elapsed().as_millis() as u64;
        // Latence économisée = verifier_latency - total_latency_ms.
        // Si on a "fini" plus vite que ce que le verifier seul aurait pris,
        // c'est négatif sur total_latency_ms (économie).  Ici on prend la
        // différence entre baseline (verifier seul) et observée.
        let latency_saved_ms = verifier_latency - total_latency_ms as i64;

        Ok(SpeculationOutcome {
            final_text,
            draft_text: draft.text,
            draft_accepted,
            tokens_accepted,
            tokens_regenerated,
            latency_saved_ms,
            latency_ms: total_latency_ms,
            verifier_id: self.verifier.id(),
        })
    }
}

/// Similarité Jaccard mot-à-mot (utile pour tester si draft ≈ verify).
fn jaccard_words(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let sa: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let sb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count().max(1);
    inter as f32 / union as f32
}

/// Compte les tokens du draft "préservés" dans le verify (préfixe commun en mots).
fn tokens_accepted_count(draft: &str, verify: &str) -> u32 {
    let d: Vec<&str> = draft.split_whitespace().collect();
    let v: Vec<&str> = verify.split_whitespace().collect();
    let mut n = 0u32;
    for (a, b) in d.iter().zip(v.iter()) {
        if a == b {
            n += 1;
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmProvider;

    #[test]
    fn jaccard_identical_is_one() {
        assert!((jaccard_words("a b c", "a b c") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn jaccard_disjoint_is_zero() {
        assert!(jaccard_words("a b", "c d") < 1e-6);
    }

    #[test]
    fn jaccard_half_overlap() {
        // a,b ∩ b,c = {b} ; a,b ∪ b,c = {a,b,c} → 1/3.
        let s = jaccard_words("a b", "b c");
        assert!((s - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn tokens_accepted_handles_empty() {
        assert_eq!(tokens_accepted_count("", "anything"), 0);
        assert_eq!(tokens_accepted_count("anything", ""), 0);
    }

    #[test]
    fn tokens_accepted_counts_prefix() {
        assert_eq!(tokens_accepted_count("a b c", "a b X"), 2);
        assert_eq!(tokens_accepted_count("a b c", "a b c d"), 3);
    }

    #[tokio::test]
    async fn speculation_accepts_when_draft_matches() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("the answer is 42"));
        let verifier = Arc::new(MockLlmProvider::new("verifier").with_response("the answer is 42"));
        let s = Speculator::new(drafter, verifier, 0.5);
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();
        assert!(out.draft_accepted);
        assert_eq!(out.tokens_regenerated, 0);
    }

    #[tokio::test]
    async fn speculation_rejects_when_draft_diverges() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("totally wrong answer"));
        let verifier = Arc::new(MockLlmProvider::new("verifier").with_response("correct ground truth response"));
        let s = Speculator::new(drafter, verifier, 0.9);
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();
        assert!(!out.draft_accepted);
        assert_eq!(out.final_text, "correct ground truth response");
    }

    #[tokio::test]
    async fn speculation_ids_match() {
        let drafter = Arc::new(MockLlmProvider::new("d"));
        let verifier = Arc::new(MockLlmProvider::new("v"));
        let s = Speculator::new(drafter, verifier, 0.5);
        assert_eq!(s.drafter_id(), "d");
        assert_eq!(s.verifier_id(), "v");
    }
}
