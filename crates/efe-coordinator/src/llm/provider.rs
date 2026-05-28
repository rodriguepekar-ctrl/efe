// SPDX-License-Identifier: UNLICENSED
//! Trait `LlmProvider` + types I/O — surface unifiée Claude/GPT-4/Gemini.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::CoordResult;

/// Identifiant kebab-case d'un provider (`"claude_opus_4"`, `"gpt4_turbo"`).
pub type ProviderId = String;

/// Requête unifiée envoyée à un LLM gros.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCall {
    /// Prompt complet (peut être déjà pré-mâché par les nano-robots).
    pub prompt: String,
    /// Tokens max à générer.
    pub max_tokens: u32,
    /// Température 0.0..2.0.
    pub temperature: f32,
    /// Estampille de deadline (None = pas de cap).
    #[serde(default)]
    pub deadline: Option<Duration>,
    /// Tag de traçabilité (forwardé dans les évènements).
    #[serde(default)]
    pub trace_tag: Option<String>,
}

impl LlmCall {
    /// Construit un call simple.
    #[must_use]
    pub fn simple(prompt: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            prompt: prompt.into(),
            max_tokens,
            temperature: 0.7,
            deadline: None,
            trace_tag: None,
        }
    }
}

/// Réponse complète d'un LLM (collectée après streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Texte concaténé.
    pub text: String,
    /// Tokens d'output produits.
    pub tokens_generated: u32,
    /// Coût observé en cents (peut être estimé si provider ne facture pas).
    ///
    /// **R3.S1** : ce champ est désormais **dérivé** de `cost_microcents`
    /// via [`microcents_to_cents`] à la construction. Préservé pour
    /// backward-compat 100 % avec les consumers pré-R3.
    ///
    /// ⚠ Résolution insuffisante sur fixtures courtes (< 200 chars) : tous
    /// les coûts s'arrondissent à 1-2 ¢. Préférer [`Self::cost_microcents`]
    /// pour reporting précis.
    pub cost_cents: u32,
    /// **Sprint R3.S1** — Coût exact en microcents (1 ¢ = 1000 μ¢).
    ///
    /// Précision ×1000 vs `cost_cents`. Permet de distinguer Claude
    /// (~45 μ¢ sur fixture courte) vs Gemini Flash (~1 μ¢) — un ratio ×46
    /// invisible en u32 cents.
    ///
    /// `#[serde(default)]` : les JSON pré-R3 sans ce champ désérialisent
    /// avec `cost_microcents = 0`. Voir [`Self::cost_microcents_resolved`]
    /// pour une lecture robuste qui reconstruit depuis `cost_cents` si
    /// `cost_microcents == 0` (perte ×1000 précision sur l'historique).
    #[serde(default)]
    pub cost_microcents: u64,
    /// Latence end-to-end ms.
    pub latency_ms: u64,
    /// Provider qui a répondu.
    pub provider: ProviderId,
    /// Score qualité auto-estimé (utilisé par le vote ; defaults to 1.0).
    #[serde(default = "default_quality")]
    pub self_quality: f32,
}

impl LlmResponse {
    /// **R3.S1** — Lecture robuste de `cost_microcents` qui reconstruit
    /// depuis `cost_cents × 1000` si le JSON est pré-R3 (perte ×1000
    /// précision mais valeur non-nulle).
    ///
    /// À utiliser par les consumers de reporting (SavingsReport, Prometheus)
    /// qui veulent traiter uniformément les records R2 et R3.
    #[must_use]
    pub fn cost_microcents_resolved(&self) -> u64 {
        if self.cost_microcents == 0 && self.cost_cents > 0 {
            cents_to_microcents(self.cost_cents)
        } else {
            self.cost_microcents
        }
    }
}

const fn default_quality() -> f32 {
    1.0
}

// ─────────────────────────────────────────────────────────────────────────
// R3.S1 — Helpers cost_microcents ↔ cost_cents
// ─────────────────────────────────────────────────────────────────────────

/// **R3.S1** — Convertit `cost_microcents` (u64) → `cost_cents` (u32).
///
/// Politique de rounding **half-up avec floor minimum 1** : si l'entrée
/// est strictement > 0, le résultat est strictement ≥ 1. Cela préserve
/// la sémantique pré-R3 "always show 1 cent if any real cost" qui sécurise
/// les tests R2 existants qui assertent `cost_cents >= 1`.
///
/// ## Exemples
///
/// - `0 microcents → 0 cents` (vraiment gratuit)
/// - `1 microcent → 1 cent` (politique floor : minimum 1 si > 0)
/// - `499 microcents → 1 cent` (idem, < 500 mais > 0)
/// - `500 microcents → 1 cent` (half-up)
/// - `1_499 microcents → 1 cent`
/// - `1_500 microcents → 2 cents`
/// - `45_000 microcents → 45 cents` (= 0.45 $)
#[must_use]
pub const fn microcents_to_cents(mc: u64) -> u32 {
    if mc == 0 {
        return 0;
    }
    // saturating_add évite l'overflow quand mc est proche de u64::MAX.
    let rounded = mc.saturating_add(500) / 1000;
    let clamped = if rounded < 1 { 1 } else { rounded };
    if clamped > u32::MAX as u64 {
        u32::MAX
    } else {
        clamped as u32
    }
}

/// **R3.S1** — Convertit `cost_cents` (u32) → `cost_microcents` (u64).
///
/// Pas de rounding : `c × 1000`. Utilisé pour migration backward-compat
/// (récupérer un microcents approximatif depuis un ancien record JSON).
#[must_use]
pub const fn cents_to_microcents(c: u32) -> u64 {
    (c as u64) * 1000
}

/// **R3.S1** — Calcul exact du coût en microcents à partir des tokens
/// observés et des prix `microcents/M` du provider.
///
/// Algorithme stable : multiplication d'abord, division par 1M en dernier
/// pour préserver la précision microcent. `saturating_mul` + `saturating_add`
/// protègent contre overflow extrême (queries > 10M tokens).
///
/// ## Formule
///
/// ```text
/// cost_microcents = (input_tokens × cost_input_per_million_microcents) / 1_000_000
///                 + (output_tokens × cost_output_per_million_microcents) / 1_000_000
/// ```
///
/// ## Exemple Claude Sonnet 4.6 (pricing 300_000 / 1_500_000 μ¢/M)
///
/// Sur 50 input + 20 output tokens :
/// - input  = 50 × 300_000 / 1M = 15 μ¢
/// - output = 20 × 1_500_000 / 1M = 30 μ¢
/// - total  = **45 μ¢** (= 0.045 ¢, invisible en u32 cents)
#[must_use]
pub fn compute_cost_microcents(
    input_tokens: u32,
    output_tokens: u32,
    cost_input_per_million_microcents: u64,
    cost_output_per_million_microcents: u64,
) -> u64 {
    let in_mc = (input_tokens as u64)
        .saturating_mul(cost_input_per_million_microcents)
        / 1_000_000;
    let out_mc = (output_tokens as u64)
        .saturating_mul(cost_output_per_million_microcents)
        / 1_000_000;
    in_mc.saturating_add(out_mc)
}

/// Un chunk émis pendant le streaming d'un LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenChunk {
    /// Texte du chunk.
    pub token: String,
    /// Index séquentiel dans la réponse.
    pub index: u64,
    /// Estampille de production.
    pub emitted_at: chrono::DateTime<chrono::Utc>,
}

/// Trait abstrait : tout ce qui peut répondre à un `LlmCall`.
///
/// La granularité est : 1 trait impl = 1 provider en mémoire.
/// L'orchestration (round-robin, retry, fallback) se fait dans
/// [`crate::llm::MultiLlmBridge`].
#[async_trait]
pub trait LlmProvider: Send + Sync + 'static {
    /// Identifiant kebab-case.
    fn id(&self) -> ProviderId;

    /// Appel non-streaming : retourne la réponse complète.
    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse>;

    /// Appel streaming : envoie chaque token sur le canal `tx`.
    ///
    /// Default : implémentation naïve via `call()` qui découpe sur les
    /// espaces (utile pour tests ; providers réels override).
    async fn stream(
        &self,
        req: LlmCall,
        tx: mpsc::Sender<TokenChunk>,
    ) -> CoordResult<LlmResponse> {
        let resp = self.call(req).await?;
        for (i, tok) in resp.text.split_whitespace().enumerate() {
            let chunk = TokenChunk {
                token: format!("{tok} "),
                index: i as u64,
                emitted_at: chrono::Utc::now(),
            };
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        Ok(resp)
    }

    /// Health check — utilisé au boot du Coordinator.
    async fn health(&self) -> CoordResult<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_call_simple_defaults() {
        let c = LlmCall::simple("hi", 100);
        assert_eq!(c.prompt, "hi");
        assert_eq!(c.max_tokens, 100);
        assert!((c.temperature - 0.7).abs() < 1e-6);
        assert!(c.deadline.is_none());
    }

    #[test]
    fn llm_response_roundtrip() {
        let r = LlmResponse {
            text: "x".into(),
            tokens_generated: 10,
            cost_cents: 1,
            cost_microcents: 1_000,
            latency_ms: 50,
            provider: "claude".into(),
            self_quality: 0.95,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: LlmResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back.tokens_generated, 10);
        assert_eq!(back.cost_microcents, 1_000);
    }

    #[test]
    fn token_chunk_roundtrip() {
        let c = TokenChunk {
            token: "hi".into(),
            index: 0,
            emitted_at: chrono::Utc::now(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: TokenChunk = serde_json::from_str(&s).unwrap();
        assert_eq!(back.token, "hi");
    }

    // ─── Sprint R3.S1 — Helpers cost_microcents / cost_cents ────────────

    #[test]
    fn r3s1_microcents_to_cents_zero_stays_zero() {
        assert_eq!(microcents_to_cents(0), 0);
    }

    #[test]
    fn r3s1_microcents_to_cents_floor_min_1_when_positive() {
        // Politique "always show 1 cent if any real cost" préserve les tests R2.
        assert_eq!(microcents_to_cents(1), 1);
        assert_eq!(microcents_to_cents(45), 1, "45 μ¢ (Claude fixture 01) → 1 ¢");
        assert_eq!(microcents_to_cents(499), 1);
    }

    #[test]
    fn r3s1_microcents_to_cents_rounds_half_up_above_1500() {
        assert_eq!(microcents_to_cents(500), 1);
        assert_eq!(microcents_to_cents(1_499), 1);
        assert_eq!(microcents_to_cents(1_500), 2);
        assert_eq!(microcents_to_cents(45_000), 45);
        assert_eq!(microcents_to_cents(100_000), 100);
    }

    #[test]
    fn r3s1_microcents_to_cents_saturates_at_u32_max() {
        let huge = u64::MAX;
        let result = microcents_to_cents(huge);
        assert_eq!(result, u32::MAX);
    }

    #[test]
    fn r3s1_cents_to_microcents_multiplies_by_1000() {
        assert_eq!(cents_to_microcents(0), 0);
        assert_eq!(cents_to_microcents(1), 1_000);
        assert_eq!(cents_to_microcents(45), 45_000);
        assert_eq!(cents_to_microcents(u32::MAX), (u32::MAX as u64) * 1000);
    }

    #[test]
    fn r3s1_compute_cost_microcents_claude_fixture_01() {
        // Claude Sonnet 4.6 : 300_000 input + 1_500_000 output (μ¢/M)
        // Fixture 01 typique : ~50 input tokens + ~20 output tokens
        // Expected : 50 × 300_000 / 1M + 20 × 1_500_000 / 1M = 15 + 30 = 45 μ¢
        let mc = compute_cost_microcents(50, 20, 300_000, 1_500_000);
        assert_eq!(mc, 45, "Claude fixture 01 doit coûter exactement 45 μ¢");
    }

    #[test]
    fn r3s1_compute_cost_microcents_gemini_with_100_tokens() {
        // Gemini Flash : 7_500 input + 30_000 output (μ¢/M EXACT, vs 8/30 arrondi R2)
        // 100 input + 50 output :
        // 100 × 7_500 / 1M = 0 (int div)
        // 50 × 30_000 / 1M = 1
        let mc = compute_cost_microcents(100, 50, 7_500, 30_000);
        assert_eq!(mc, 1, "Gemini Flash 100+50 tokens ≈ 1 μ¢");
    }

    #[test]
    fn r3s1_compute_cost_microcents_overflow_safe() {
        // u32::MAX tokens × u64::MAX μ¢/M → saturating, pas de panic
        let mc = compute_cost_microcents(u32::MAX, u32::MAX, u64::MAX, u64::MAX);
        assert!(mc <= u64::MAX);
    }

    #[test]
    fn r3s1_compute_cost_microcents_zero_when_no_tokens() {
        let mc = compute_cost_microcents(0, 0, 300_000, 1_500_000);
        assert_eq!(mc, 0);
    }

    #[test]
    fn r3s1_llm_response_cost_microcents_resolved_uses_microcents_when_set() {
        let r = LlmResponse {
            text: "x".into(),
            tokens_generated: 10,
            cost_cents: 1,
            cost_microcents: 45,
            latency_ms: 50,
            provider: "claude".into(),
            self_quality: 0.95,
        };
        assert_eq!(r.cost_microcents_resolved(), 45);
    }

    #[test]
    fn r3s1_llm_response_cost_microcents_resolved_reconstructs_from_cents() {
        // Cas pré-R3 : JSON avait cost_cents=1 mais pas cost_microcents.
        // serde_default → cost_microcents=0.
        // resolved() doit reconstruire 1 × 1000 = 1000 μ¢ (perte de précision documentée).
        let r = LlmResponse {
            text: "x".into(),
            tokens_generated: 10,
            cost_cents: 1,
            cost_microcents: 0, // pre-R3 default
            latency_ms: 50,
            provider: "claude".into(),
            self_quality: 0.95,
        };
        assert_eq!(r.cost_microcents_resolved(), 1_000);
    }

    #[test]
    fn r3s1_llm_response_backward_compat_pre_r3_json() {
        // JSON pré-R3 sans cost_microcents
        let old_json = r#"{
            "text": "Paris",
            "tokens_generated": 1,
            "cost_cents": 1,
            "latency_ms": 1000,
            "provider": "claude",
            "self_quality": 1.0
        }"#;
        let parsed: LlmResponse = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.cost_cents, 1);
        assert_eq!(parsed.cost_microcents, 0, "default serde");
        assert_eq!(parsed.cost_microcents_resolved(), 1_000);
    }
}
