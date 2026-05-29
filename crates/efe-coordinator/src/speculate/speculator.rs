// SPDX-License-Identifier: UNLICENSED
//! Implémentation `Speculator`.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::CoordResult;
use crate::events::{EventBus, StreamEvent};
use crate::llm::{LlmCall, LlmProvider, TokenChunk};
// R3.S3 (fix L4/D6) : gate de rejet de bruit pour le hot path.
use super::noise::{NoiseRejectionPolicy, NoiseSignal};
// R3.S4 : circuit breaker pour le drafter SLM local.
use efe_nano_app_coordinator::{SlmCircuitBreaker, SlmCircuitState};

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

    /// **R3.S1** : coût total en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// Particulièrement précieux pour Speculative HOT PATH : le drafter
    /// Gemini Flash coûte en pratique quelques dizaines de µ¢ (e.g. 37 µ¢
    /// pour 1k/1k tokens), invisible dans `cost_cents` (arrondi à 1).
    /// L'agrégation sur N missions devient honnête.
    ///
    /// `#[serde(default)]` pour backward-compat avec outcomes pré-R3.
    #[serde(default)]
    pub cost_microcents: u64,

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

    /// **R3.S3 (fix L4/D6)** — Signal de bruit ayant bloqué le hot path.
    ///
    /// - `None` : hot path pris (draft propre), OU slow path nominal
    ///   (draft propre mais > `max_words`), OU drafter bypassed.
    /// - `Some(signal)` : le draft a été détecté comme bruit (« La »,
    ///   refus, répétition…) → hot path bloqué → slow path forcé → le
    ///   verifier a produit la vraie réponse.
    ///
    /// `#[serde(default)]` pour backward-compat avec outcomes pré-R3.S3.
    #[serde(default)]
    pub noise_signal: Option<NoiseSignal>,
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
    /// **R3.S3 (fix L4/D6)** — Politique de rejet de bruit appliquée AVANT
    /// la décision hot path. Un draft détecté comme bruit (« La », refus,
    /// répétition) est forcé sur le slow path (verifier). Défaut :
    /// [`NoiseRejectionPolicy::default`] (gate actif). Utiliser
    /// [`permissive_s7`](NoiseRejectionPolicy::permissive_s7) pour
    /// désactiver (backward-compat S7).
    noise_policy: NoiseRejectionPolicy,
    /// **R3.S4** — Circuit breaker optionnel pour le drafter (typiquement un
    /// SLM local qui peut crasher/OOM). Si `Some` et ouvert, le drafter est
    /// **skippé** avant même l'appel (évite le timeout sur un llama-server
    /// mort) → verifier-only (drafter_bypassed). `None` = comportement S8
    /// (bypass seulement après échec/timeout réel).
    drafter_circuit: Option<SlmCircuitBreaker>,
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
            // R3.S3 : gate de bruit actif par défaut (sans effet tant que le
            // hot path est désactivé, mais prêt dès `with_quick_accept`).
            noise_policy: NoiseRejectionPolicy::default(),
            // R3.S4 : pas de circuit drafter par défaut (comportement S8).
            drafter_circuit: None,
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

    /// **R3.S3 (fix L4/D6)** — Configure la politique de rejet de bruit.
    ///
    /// Le gate s'applique AVANT la décision hot path : un draft détecté
    /// comme bruit (mot-de-contenu nul, refus, répétition dégénérée) est
    /// forcé sur le slow path où le verifier produit la vraie réponse.
    ///
    /// Exemple :
    /// ```ignore
    /// Speculator::new(d, v, 0.7)
    ///     .with_quick_accept(10)
    ///     .with_noise_policy(NoiseRejectionPolicy::default());
    /// ```
    #[must_use]
    pub fn with_noise_policy(mut self, policy: NoiseRejectionPolicy) -> Self {
        self.noise_policy = policy;
        self
    }

    /// **R3.S4** — Attache un circuit breaker pour le drafter.
    ///
    /// Si le circuit est ouvert (drafter SLM local répétitivement en
    /// échec), le `run()` skippe le drafter et passe directement au
    /// verifier (drafter_bypassed) — évitant le timeout sur un
    /// llama-server mort. Réutilise la primitive R3.S2.
    #[must_use]
    pub fn with_drafter_circuit(mut self, circuit: SlmCircuitBreaker) -> Self {
        self.drafter_circuit = Some(circuit);
        self
    }

    /// **R3.S4** — Variante `Option` (ergonomie wiring workflow : le
    /// circuit peut être absent si le drafter n'en a pas).
    #[must_use]
    pub fn with_drafter_circuit_opt(mut self, circuit: Option<SlmCircuitBreaker>) -> Self {
        self.drafter_circuit = circuit;
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

    /// **R3.S4** — Fallback verifier-only : le drafter a été skippé
    /// (circuit ouvert) ou a échoué. Émet `DrafterBypassed` puis appelle
    /// le verifier directement. Facteur commun (DRY) entre le guard circuit
    /// (Step 0) et la branche d'échec drafter (Step 1).
    ///
    /// `drafter_latency_ms` = 0 si skip pré-call, sinon la latence observée
    /// avant l'échec.
    async fn verifier_only_fallback(
        &self,
        req: LlmCall,
        started: Instant,
        drafter_latency_ms: u64,
        bypass_reason: &str,
    ) -> CoordResult<SpeculationOutcome> {
        if let (Some(bus), Some(swarm)) = (self.bus.as_ref(), self.swarm_id) {
            let _ = bus
                .publish(StreamEvent::DrafterBypassed {
                    swarm_id: swarm,
                    drafter: self.drafter.id(),
                    fallback_to_verifier: self.verifier.id(),
                    reason: bypass_reason.to_string(),
                    ts: chrono::Utc::now(),
                })
                .await;
        }
        let verifier_started = Instant::now();
        let verify_res = self.verifier.call(req).await?;
        let verifier_latency_ms = verifier_started.elapsed().as_millis() as u64;
        Ok(SpeculationOutcome {
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
            cost_microcents: verify_res.cost_microcents_resolved(),
            drafter_bypassed: true,
            noise_signal: None,
        })
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
        let bus_opt = self.bus.clone();
        let swarm_opt = self.swarm_id;

        // ── Step 0 (R3.S4) : drafter circuit guard ──
        //
        // Si le circuit du drafter (SLM local) est ouvert, on SKIP le
        // drafter avant même de l'appeler (évite le timeout sur un
        // llama-server mort) → verifier-only (drafter_bypassed).
        if let Some(cb) = &self.drafter_circuit {
            if cb.is_open() {
                return self
                    .verifier_only_fallback(req, started, 0, "drafter_circuit_open")
                    .await;
            }
        }

        // ── Step 1 : Drafter d'abord (séquentiel pour permettre hot path) ──
        //
        // S8 R2 patch : si le drafter fail, on bascule sur le verifier
        // direct au lieu de propager l'erreur. Trace via `drafter_bypassed`.
        let drafter_started = Instant::now();
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
        // R3.S4 : record success/failure sur le circuit drafter.
        let draft = match draft_result {
            Ok(d) => {
                if let Some(cb) = &self.drafter_circuit {
                    cb.record_success();
                }
                d
            }
            Err(drafter_err) => {
                // R3.S4 : record_failure + emit SlmCircuitOpened si transition.
                if let Some(cb) = &self.drafter_circuit {
                    let was_closed = cb.state() == SlmCircuitState::Closed;
                    cb.record_failure();
                    if was_closed && cb.is_open() {
                        if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
                            let _ = bus
                                .publish(StreamEvent::SlmCircuitOpened {
                                    swarm_id: swarm,
                                    robot: self.drafter.id(),
                                    failures: cb.failure_count() as u32,
                                    ts: chrono::Utc::now(),
                                })
                                .await;
                        }
                    }
                }
                // Verifier-only fallback (factorisé, DRY avec Step 0).
                return self
                    .verifier_only_fallback(
                        req,
                        started,
                        drafter_latency_ms,
                        &drafter_err.to_string(),
                    )
                    .await;
            }
        };

        // ── Step 2 : Décision Quick-Accept (HOT PATH) ──
        //
        // Si `max_words_for_quick_accept > 0` ET la réponse drafter respecte
        // ce seuil ET n'est PAS du bruit → on accepte le draft SANS appeler
        // le verifier. C'est l'optimisation économique fondamentale de S7.
        //
        // R3.S3 (fix L4/D6) — sémantique PRÉCISE du « noise rejection » :
        // le gate ne s'applique QUE quand le draft était un CANDIDAT au hot
        // path (hot path activé ET draft assez court). Un draft trop long
        // OU le mode slow-path-only (max_words=0) ne passe jamais par le hot
        // path → la détection de bruit y serait du bruit télémétrique (on
        // signalerait une « rejection » alors que rien n'aurait été
        // hot-accepté). On ne détecte/émet donc QUE quand le bruit FLIP un
        // hot-accept qui aurait sinon eu lieu. C'est exactement le problème
        // L4/D6 : « du bruit était hot-accepté à tort ».
        let draft_words = draft.text.split_whitespace().count();
        let is_hot_path_candidate = self.max_words_for_quick_accept > 0
            && draft_words <= self.max_words_for_quick_accept;
        let noise = if is_hot_path_candidate {
            self.noise_policy.detect(&draft.text, &req.prompt)
        } else {
            None // hors hot path → bruit non pertinent (pas de rejection)
        };
        let hot_path = is_hot_path_candidate && noise.is_none();

        // R3.S3 : observabilité — si le bruit a bloqué un hot-accept qui
        // aurait sinon eu lieu, émet l'event (le draft retombe sur le slow
        // path où le verifier produit la vraie réponse).
        if let Some(signal) = noise {
            if let (Some(bus), Some(swarm)) = (bus_opt.as_ref(), swarm_opt) {
                let excerpt: String = draft.text.chars().take(40).collect();
                let _ = bus
                    .publish(StreamEvent::DraftRejectedAsNoise {
                        swarm_id: swarm,
                        signal: signal.as_str().to_string(),
                        draft_excerpt: excerpt,
                        ts: chrono::Utc::now(),
                    })
                    .await;
            }
        }

        if hot_path {
            let total_latency_ms = started.elapsed().as_millis() as u64;
            // R3.S1 : précompute avant le partial-move de draft.text.
            let cost_microcents = draft.cost_microcents_resolved();
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
                // R3.S1 : précision µ¢ — HOT PATH seul drafter facturé.
                // C'est ici que la précision µ¢ paie le plus :
                // Gemini Flash 1k/1k tokens = 37 µ¢ EXACT (vs 1 cent
                // arrondi sur cost_cents).
                cost_microcents,
                // S8 R2 : hot path nominal — drafter a réussi.
                drafter_bypassed: false,
                // R3.S3 : hot path pris → draft propre → pas de bruit.
                noise_signal: None,
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
        // R3.S1 : précompute avant le partial-move sur draft.text et verify_res.
        let cost_cents = draft.cost_cents.saturating_add(verify_res.cost_cents);
        let cost_microcents = draft
            .cost_microcents_resolved()
            .saturating_add(verify_res.cost_microcents_resolved());

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
            cost_cents,
            // R3.S1 : précision µ¢ — SLOW PATH = drafter + verifier.
            cost_microcents,
            // S8 R2 : slow path nominal — drafter a réussi.
            drafter_bypassed: false,
            // R3.S3 : slow path par bruit → `Some(signal)` ; slow path par
            // longueur (draft propre > max_words) → `None`.
            noise_signal: noise,
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
    use std::time::Duration;

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

    // ================================================================
    // R3.S3 — Tests noise rejection (fix L4/D6)
    // ================================================================

    /// **R3.S3 (le fix D6)** : le drafter renvoie « La » (bruit, 0 mot de
    /// contenu). Le hot path doit être BLOQUÉ → slow path → le verifier
    /// produit « Paris ». Sans R3.S3, « La » aurait été livré tel quel.
    #[tokio::test]
    async fn r3s3_speculator_rejects_la_noise_falls_to_slow_path() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("La"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_noise_policy(NoiseRejectionPolicy::default());
        let out = s.run(LlmCall::simple("Capitale de la France ?", 10)).await.unwrap();

        assert!(
            !out.verifier_skipped,
            "« La » est du bruit → hot path bloqué → verifier appelé"
        );
        assert_eq!(out.final_text, "Paris", "le verifier produit la vraie réponse");
        assert_eq!(
            out.noise_signal,
            Some(NoiseSignal::OnlyStopwords),
            "« La » détecté comme OnlyStopwords"
        );
    }

    /// **R3.S3 (anti-régression)** : « Paris » (1 mot de contenu) DOIT
    /// continuer à prendre le hot path. C'est le test qui prouve qu'on ne
    /// détruit PAS le savings sur le cas FactualQa canonique.
    #[tokio::test]
    async fn r3s3_speculator_accepts_paris_hot_path() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("Paris"));
        let verifier = Arc::new(
            MockLlmProvider::new("verifier").with_response("Paris is the capital"),
        );
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_noise_policy(NoiseRejectionPolicy::default());
        let out = s.run(LlmCall::simple("Capital of France?", 10)).await.unwrap();

        assert!(out.verifier_skipped, "« Paris » propre → hot path → verifier skip");
        assert_eq!(out.final_text, "Paris");
        assert_eq!(out.noise_signal, None, "pas de bruit");
        assert!(out.draft_accepted);
    }

    /// **R3.S3** : un refus (« I don't know ») est rejeté → slow path.
    #[tokio::test]
    async fn r3s3_speculator_rejects_refusal() {
        let drafter =
            Arc::new(MockLlmProvider::new("drafter").with_response("I don't know"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_noise_policy(NoiseRejectionPolicy::default());
        let out = s.run(LlmCall::simple("Capital?", 10)).await.unwrap();
        assert!(!out.verifier_skipped);
        assert_eq!(out.final_text, "Paris");
        assert_eq!(out.noise_signal, Some(NoiseSignal::RefusalPattern));
    }

    /// **R3.S3 (opt-out)** : avec `permissive_s7`, « La » reprend le hot
    /// path (comportement S7 exact restauré).
    #[tokio::test]
    async fn r3s3_permissive_s7_accepts_la_again() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("La"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_noise_policy(NoiseRejectionPolicy::permissive_s7());
        let out = s.run(LlmCall::simple("q", 10)).await.unwrap();
        assert!(out.verifier_skipped, "permissive_s7 → « La » prend le hot path");
        assert_eq!(out.final_text, "La");
        assert_eq!(out.noise_signal, None);
    }

    /// **R3.S3 (sémantique précise)** : un draft-bruit TROP LONG (> max_words)
    /// n'est PAS flaggé comme noise — il n'était pas candidat au hot path
    /// (rejeté pour longueur, pas pour bruit). `noise_signal` doit rester
    /// `None` (il va au slow path pour cause de longueur).
    #[tokio::test]
    async fn r3s3_long_noisy_draft_not_flagged_as_noise() {
        // 12 « La » > max_words 10 → jamais candidat hot path.
        let long_noise = "La ".repeat(12);
        let drafter =
            Arc::new(MockLlmProvider::new("drafter").with_response(long_noise.trim()));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_noise_policy(NoiseRejectionPolicy::default());
        let out = s.run(LlmCall::simple("q", 10)).await.unwrap();
        assert!(!out.verifier_skipped, "draft long → slow path (longueur)");
        assert_eq!(
            out.noise_signal, None,
            "draft trop long → rejeté pour LONGUEUR, pas pour bruit → noise_signal None"
        );
    }

    /// **R3.S3 (sémantique précise)** : en mode slow-path-only (max_words=0,
    /// hot path désactivé), un draft-bruit n'est PAS flaggé — le hot path
    /// n'était pas une option, donc aucune « rejection » de bruit n'a lieu.
    #[tokio::test]
    async fn r3s3_slow_path_only_mode_does_not_flag_noise() {
        let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("La"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        // PAS de with_quick_accept → max_words=0 → hot path off.
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_noise_policy(NoiseRejectionPolicy::default());
        let out = s.run(LlmCall::simple("q", 10)).await.unwrap();
        assert!(!out.verifier_skipped, "max_words=0 → toujours slow path");
        assert_eq!(
            out.noise_signal, None,
            "hot path désactivé → pas de candidat → noise_signal None"
        );
    }

    // ================================================================
    // R3.S4 — Tests drafter circuit breaker (résilience SLM local)
    // ================================================================

    /// **R3.S4** : circuit drafter pré-ouvert → le drafter n'est PAS appelé
    /// → verifier-only (drafter_bypassed). Le drafter mock paniquerait s'il
    /// était appelé (with_error), prouvant le skip.
    #[tokio::test]
    async fn r3s4_drafter_circuit_open_skips_to_verifier() {
        // Drafter qui FAIL toujours (ne devrait jamais être atteint).
        let drafter =
            Arc::new(MockLlmProvider::new("local_slm").with_error("should not be called"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        cb.record_failure(); // → Open
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_drafter_circuit(cb);
        let out = s.run(LlmCall::simple("Capital?", 10)).await.unwrap();

        assert!(out.drafter_bypassed, "circuit open → drafter bypassed");
        assert!(!out.verifier_skipped, "verifier appelé en fallback");
        assert_eq!(out.final_text, "Paris");
        assert_eq!(out.drafter_latency_ms, 0, "drafter skippé → latence 0");
    }

    /// **R3.S4** : un échec drafter enregistre un failure sur le circuit +
    /// bypass vers le verifier (la mission produit quand même la réponse).
    #[tokio::test]
    async fn r3s4_drafter_failure_records_and_bypasses() {
        let drafter =
            Arc::new(MockLlmProvider::new("local_slm").with_error("llama-server down"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        let cb = SlmCircuitBreaker::new(3, Duration::from_secs(10));
        let cb_handle = cb.clone();
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_drafter_circuit(cb);
        let out = s.run(LlmCall::simple("Capital?", 10)).await.unwrap();

        assert!(out.drafter_bypassed, "drafter fail → bypass");
        assert_eq!(out.final_text, "Paris", "verifier produit la réponse");
        assert_eq!(
            cb_handle.failure_count(),
            1,
            "1 échec drafter → 1 failure enregistré sur le circuit"
        );
    }

    /// **R3.S4** : un succès drafter enregistre un success (circuit reste
    /// Closed) et le hot path fonctionne normalement.
    #[tokio::test]
    async fn r3s4_drafter_success_records_success() {
        let drafter =
            Arc::new(MockLlmProvider::new("local_slm").with_response("Paris"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris is capital"));
        let cb = SlmCircuitBreaker::new(3, Duration::from_secs(10));
        let cb_handle = cb.clone();
        // Pré-charge 1 failure pour vérifier que le success reset.
        cb_handle.record_failure();
        let s = Speculator::new(drafter, verifier, 0.5)
            .with_quick_accept(10)
            .with_drafter_circuit(cb);
        let out = s.run(LlmCall::simple("Capital?", 10)).await.unwrap();

        assert!(out.verifier_skipped, "« Paris » propre → hot path");
        assert_eq!(out.final_text, "Paris");
        assert_eq!(
            cb_handle.failure_count(),
            0,
            "succès drafter → record_success reset le compteur"
        );
    }

    /// **R3.S4** : sans circuit drafter, comportement S8 préservé (bypass
    /// seulement après échec réel, pas de skip proactif).
    #[tokio::test]
    async fn r3s4_no_drafter_circuit_keeps_s8_behavior() {
        let drafter =
            Arc::new(MockLlmProvider::new("local_slm").with_error("down"));
        let verifier =
            Arc::new(MockLlmProvider::new("verifier").with_response("Paris"));
        // PAS de with_drafter_circuit.
        let s = Speculator::new(drafter, verifier, 0.5).with_quick_accept(10);
        let out = s.run(LlmCall::simple("Capital?", 10)).await.unwrap();
        // S8 : le drafter est appelé, échoue, puis bypass.
        assert!(out.drafter_bypassed);
        assert_eq!(out.final_text, "Paris");
    }
}
