// SPDX-License-Identifier: UNLICENSED
//! Implémentation `Speculator`.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::CoordResult;
use crate::events::{EventBus, StreamEvent};
use crate::llm::{LlmCall, LlmProvider, TokenChunk};

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

    /// **Sprint S7 R2** — Le verifier API a-t-il été appelé ?
    ///
    /// `true` (HOT PATH) : le drafter a suffi (réponse courte respectant
    /// `max_words_for_quick_accept`) → l'API verifier n'a JAMAIS été touchée
    /// → coût et latence du verifier non payés.
    ///
    /// `false` (SLOW PATH) : verifier appelé pour comparaison Jaccard ; le
    /// draft peut quand même être "accepté" (`draft_accepted=true`) après
    /// le check Jaccard, mais on a déjà payé l'appel verifier.
    ///
    /// Distinction critique pour la quantification du savings économique :
    /// seul `verifier_skipped=true` correspond à un VRAI savings.
    ///
    /// `#[serde(default)]` pour backward-compat avec les `SpeculationOutcome`
    /// sérialisés pré-S7 (toujours dans le slow path par construction).
    #[serde(default)]
    pub verifier_skipped: bool,

    /// **Sprint S7 R2** — Latence du drafter en ms.
    #[serde(default)]
    pub drafter_latency_ms: u64,

    /// **Sprint S7 R2** — Latence du verifier en ms (0 si `verifier_skipped`).
    #[serde(default)]
    pub verifier_latency_ms: u64,

    /// **Sprint S7 R2** — Coût total constaté en cents.
    ///
    /// - HOT PATH (`verifier_skipped=true`) : `cost_cents = drafter.cost_cents`
    ///   (seul le drafter a été facturé)
    /// - SLOW PATH (`verifier_skipped=false`) : `cost_cents = drafter.cost_cents
    ///   + verifier.cost_cents` (les 2 LLMs ont été facturés)
    ///
    /// Permet de comparer empirically savings hot vs slow path :
    /// `savings_observed = slow_path_cost - hot_path_cost`.
    ///
    /// Comble l'asymétrie pré-S7 où `SpeculativeResponse` n'avait pas de
    /// `cost_cents` (vs `MissionAResponse.cost_cents` et
    /// `MissionBResponse.total_cost_cents` qui existent depuis S2).
    ///
    /// `#[serde(default)]` pour backward-compat avec les outcomes pré-S7.
    #[serde(default)]
    pub cost_cents: u32,

    /// **Sprint S8 R2** — `true` si le drafter a échoué et qu'on a sauté
    /// direct au verifier (fallback graceful). Distinct de `verifier_skipped` :
    ///
    /// - `verifier_skipped=true, drafter_bypassed=false` : HOT PATH normal
    /// - `verifier_skipped=false, drafter_bypassed=false` : SLOW PATH normal
    /// - `verifier_skipped=false, drafter_bypassed=true` : drafter a fail,
    ///   verifier a sauvé la mission (qualité préservée mais savings=0)
    ///
    /// Tracé pour reporting "% de drafter-bypass" — si > 10 %, le drafter
    /// est probablement mal choisi (e.g. SLM crashe ou Gemini rate-limited).
    ///
    /// `#[serde(default)]` pour backward-compat avec outcomes pré-S8.
    #[serde(default)]
    pub drafter_bypassed: bool,
}

/// Speculator : orchestre drafter + verifier.
pub struct Speculator {
    drafter: Arc<dyn LlmProvider>,
    verifier: Arc<dyn LlmProvider>,
    /// Seuil de similarité Jaccard au-dessus duquel le draft est accepté
    /// **après comparaison avec le verifier** (slow path).
    accept_threshold: f32,
    /// **Sprint S7 R2** — Nombre maximum de mots dans le draft pour qu'il
    /// soit accepté **sans appeler le verifier** (hot path).
    ///
    /// Heuristique : un drafter cheap (Gemini Flash) qui répond en peu de
    /// mots à une query factuelle est généralement correct. `0` désactive
    /// le hot path (rétrocompatibilité S2-S6 → toujours slow path).
    ///
    /// Empirically calibré : Gemini Flash sur "Capital of France?" → 1-3
    /// mots. Threshold 10 est conservateur (rejette les réponses longues
    /// type hallucinations narratives).
    max_words_for_quick_accept: usize,
    /// Bus optionnel pour émettre `StreamEvent::DraftToken` pendant la frappe.
    bus: Option<Arc<dyn EventBus>>,
    /// `swarm_id` à attacher aux évènements (None → pas d'émission).
    swarm_id: Option<uuid::Uuid>,
}

impl Speculator {
    /// Construit un Speculator (sans bus = aucun évènement émis).
    ///
    /// **Note S7** : `max_words_for_quick_accept = 0` (hot path désactivé) pour
    /// rester sémantiquement identique au comportement pré-S7. Utiliser
    /// [`with_quick_accept`](Self::with_quick_accept) pour activer le hot path.
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
            max_words_for_quick_accept: 0, // S7 hot path désactivé par défaut
            bus: None,
            swarm_id: None,
        }
    }

    /// **Sprint S7 R2** — Active le hot path : si le drafter répond en
    /// `<= max_words` mots, accepter sans appeler le verifier.
    ///
    /// Exemple : `Speculator::new(...).with_quick_accept(10)` accepte les
    /// réponses ≤ 10 mots du drafter directement, économisant l'appel
    /// verifier (coût + latence).
    #[must_use]
    pub fn with_quick_accept(mut self, max_words: usize) -> Self {
        self.max_words_for_quick_accept = max_words;
        self
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

    /// Lance le drafter d'abord ; si la réponse est suffisamment courte
    /// (`max_words_for_quick_accept`), retourne le draft directement
    /// **sans appeler le verifier** (HOT PATH). Sinon appelle le verifier
    /// et arbitre via similarité Jaccard (SLOW PATH).
    ///
    /// Si un bus + swarm_id sont attachés (via [`with_bus`]), chaque
    /// token drafté est émis en temps réel comme `StreamEvent::DraftToken`.
    ///
    /// ## Évolution Sprint S7 R2
    ///
    /// **Avant S7** : le verifier était TOUJOURS appelé en parallèle du
    /// drafter, même si le draft était parfait. Le `tokens_regenerated=0`
    /// était cosmétique : on avait payé l'API verifier.
    ///
    /// **Avec S7** : le drafter est appelé EN PREMIER. Si la réponse est
    /// courte (≤ `max_words_for_quick_accept`), on n'appelle PAS le
    /// verifier — vrai savings économique sur coût + latence.
    pub async fn run(&self, req: LlmCall) -> CoordResult<SpeculationOutcome> {
        let started = Instant::now();
        let drafter = Arc::clone(&self.drafter);
        let req_d = req.clone();

        // ── Step 1 : Drafter d'abord (séquentiel pour permettre hot path) ──
        //
        // S8 R2 patch : si le drafter fail, on bascule sur le verifier
        // direct au lieu de propager l'erreur. Trace via `drafter_bypassed`.
        let drafter_started = Instant::now();
        let bus_opt = self.bus.clone();
        let swarm_opt = self.swarm_id;
        let draft_result: CoordResult<crate::llm::LlmResponse> = if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
            let (chunk_tx, mut chunk_rx) =
                tokio::sync::mpsc::channel::<TokenChunk>(256);
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
        let drafter_latency_ms = drafter_started.elapsed().as_millis() as u64;

        // S8 R2 : si drafter fail → bypass + appel verifier direct.
        let draft = match draft_result {
            Ok(d) => d,
            Err(drafter_err) => {
                // Émission event DRAFTER_BYPASSED si bus attaché.
                if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
                    let _ = bus
                        .publish(StreamEvent::DrafterBypassed {
                            swarm_id: swarm,
                            drafter: self.drafter.id(),
                            fallback_to_verifier: self.verifier.id(),
                            reason: drafter_err.to_string(),
                            ts: chrono::Utc::now(),
                        })
                        .await;
                }
                // Verifier direct comme fallback graceful.
                let verifier_started = Instant::now();
                let verify_res = self.verifier.call(req).await?;
                let verifier_latency_ms =
                    verifier_started.elapsed().as_millis() as u64;
                return Ok(SpeculationOutcome {
                    final_text: verify_res.text.clone(),
                    draft_text: String::new(),
                    draft_accepted: false,
                    tokens_accepted: 0,
                    tokens_regenerated: verify_res.tokens_generated,
                    latency_saved_ms: 0,
                    latency_ms: started.elapsed().as_millis() as u64,
                    verifier_id: self.verifier.id(),
                    verifier_skipped: false,
                    drafter_latency_ms,
                    verifier_latency_ms,
                    cost_cents: verify_res.cost_cents,
                    drafter_bypassed: true,
                });
            }
        };

        // ── Step 2 : Décision Quick-Accept (HOT PATH) ──
        //
        // Si `max_words_for_quick_accept > 0` ET la réponse drafter respecte
        // ce seuil → on accepte le draft SANS appeler le verifier.
        // C'est l'optimisation économique fondamentale de S7.
        let draft_words = draft.text.split_whitespace().count();
        let hot_path = self.max_words_for_quick_accept > 0
            && draft_words > 0
            && draft_words <= self.max_words_for_quick_accept;

        if hot_path {
            let total_latency_ms = started.elapsed().as_millis() as u64;
            return Ok(SpeculationOutcome {
                final_text: draft.text.clone(),
                draft_text: draft.text,
                draft_accepted: true,
                tokens_accepted: draft.tokens_generated,
                tokens_regenerated: 0,
                // Latence économisée = verifier baseline non observé en hot
                // path. On laisse 0 (pas de baseline mesurée) ; le savings
                // est exprimé via `verifier_skipped + verifier_latency_ms=0`.
                latency_saved_ms: 0,
                latency_ms: total_latency_ms,
                verifier_id: self.verifier.id(),
                verifier_skipped: true,
                drafter_latency_ms,
                verifier_latency_ms: 0,
                // S7 R2 : hot path → seul le drafter a été facturé.
                cost_cents: draft.cost_cents,
                // S8 R2 : hot path nominal — drafter a réussi.
                drafter_bypassed: false,
            });
        }

        // ── Step 3 : SLOW PATH — verifier appelé pour arbitrage ──
        let verifier = Arc::clone(&self.verifier);
        let verifier_started = Instant::now();
        let verify_res = verifier.call(req).await?;
        let verifier_latency_ms = verifier_started.elapsed().as_millis() as u64;

        let similarity = jaccard_words(&draft.text, &verify_res.text);
        let accept = similarity >= self.accept_threshold;
        let (final_text, tokens_accepted, tokens_regenerated, draft_accepted) =
            if accept {
                (draft.text.clone(), draft.tokens_generated, 0, true)
            } else {
                let accepted_prefix = tokens_accepted_count(&draft.text, &verify_res.text);
                (
                    verify_res.text.clone(),
                    accepted_prefix,
                    verify_res.tokens_generated.saturating_sub(accepted_prefix),
                    false,
                )
            };

        let total_latency_ms = started.elapsed().as_millis() as u64;
        // Latence économisée = baseline verifier seul - latence observée.
        // En slow path on a payé verifier_latency PLUS drafter_latency, donc
        // ce nombre sera typiquement négatif (on a perdu).
        let latency_saved_ms =
            verifier_latency_ms as i64 - total_latency_ms as i64;

        Ok(SpeculationOutcome {
            final_text,
            draft_text: draft.text,
            draft_accepted,
            tokens_accepted,
            tokens_regenerated,
            latency_saved_ms,
            latency_ms: total_latency_ms,
            verifier_id: self.verifier.id(),
            verifier_skipped: false,
            drafter_latency_ms,
            verifier_latency_ms,
            // S7 R2 : slow path → drafter + verifier facturés tous les deux.
            cost_cents: draft.cost_cents.saturating_add(verify_res.cost_cents),
            // S8 R2 : slow path nominal — drafter a réussi.
            drafter_bypassed: false,
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

    // ─── Sprint S7 R2 — Tests du hot path (quick accept) ────────────────────

    #[tokio::test]
    async fn s7_hot_path_skips_verifier_when_draft_short() {
        // Drafter répond court (3 mots ≤ threshold 10).
        let drafter = Arc::new(
            MockLlmProvider::new("drafter").with_response("Paris is capital"),
        );
        // Verifier ne devrait JAMAIS être appelé.
        // On lui colle une longue réponse pour qu'elle soit distinguable du draft.
        let verifier = Arc::new(
            MockLlmProvider::new("verifier")
                .with_response("Paris is the capital of France and lots more"),
        );
        let s = Speculator::new(drafter, verifier, 0.9).with_quick_accept(10);
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();

        assert!(out.verifier_skipped, "S7 hot path : verifier skipped");
        assert_eq!(out.final_text, "Paris is capital", "draft retourné tel quel");
        assert!(out.draft_accepted);
        assert_eq!(out.tokens_regenerated, 0);
        assert_eq!(out.verifier_latency_ms, 0, "verifier non appelé → 0 ms");
        // drafter_latency_ms peut être 0 sur mock ultra-rapide — c'est OK.
    }

    #[tokio::test]
    async fn s7_slow_path_calls_verifier_when_draft_long() {
        // Drafter répond long (> 10 mots).
        let long_draft =
            "Paris is the beautiful capital of France with many monuments and culture and history that exceeds many cities around the world today";
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response(long_draft));
        let verifier = Arc::new(
            MockLlmProvider::new("verifier").with_response("Verified Paris response"),
        );
        let s = Speculator::new(drafter, verifier, 0.9).with_quick_accept(10);
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();

        assert!(!out.verifier_skipped, "S7 slow path : verifier called");
        // Le draft a >10 mots donc on appelle le verifier ; Jaccard faible
        // donc verifier text retenu.
        assert_eq!(out.final_text, "Verified Paris response");
        // drafter_latency + verifier_latency tous deux observables (peuvent
        // être 0 sur mock instant).
    }

    #[tokio::test]
    async fn s7_quick_accept_disabled_by_default_keeps_pre_s7_behavior() {
        // Pas de with_quick_accept → max_words = 0 → slow path forcé,
        // identique au comportement S2-S6.
        let drafter = Arc::new(
            MockLlmProvider::new("drafter").with_response("short answer"),
        );
        let verifier = Arc::new(
            MockLlmProvider::new("verifier").with_response("short answer"),
        );
        let s = Speculator::new(drafter, verifier, 0.5);
        // PAS de .with_quick_accept(...)
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();
        assert!(!out.verifier_skipped, "default S2-S6 : slow path");
        // Jaccard = 1.0 (identiques) → draft accepté quand même.
        assert!(out.draft_accepted);
    }

    #[tokio::test]
    async fn s7_empty_draft_falls_through_to_slow_path() {
        // Drafter vide : draft_words = 0, ne peut pas déclencher le hot path
        // (condition `draft_words > 0`).
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response(""));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("fallback"));
        let s = Speculator::new(drafter, verifier, 0.5).with_quick_accept(10);
        let out = s.run(LlmCall::simple("?", 10)).await.unwrap();
        assert!(
            !out.verifier_skipped,
            "draft vide ne doit JAMAIS skip le verifier"
        );
        assert_eq!(out.final_text, "fallback");
    }

    #[test]
    fn s7_with_quick_accept_builder_pattern() {
        let drafter = Arc::new(MockLlmProvider::new("d"));
        let verifier = Arc::new(MockLlmProvider::new("v"));
        let s = Speculator::new(drafter, verifier, 0.5).with_quick_accept(15);
        assert_eq!(s.max_words_for_quick_accept, 15);
        // Builder chainable — confirme l'ergonomie.
    }
}
