// SPDX-License-Identifier: UNLICENSED
//! `PricingTable` — calcul du coût en cents (ADR-P3R1-004).
//!
//! Source : pricing publié par Anthropic et Google (vérifié au moment du
//! commit R1 — pricing évolue, à reverifier au déploiement).
//!
//! Représentation : `(input_cents_per_million, output_cents_per_million)`
//! pour rester en entier (pas de drift float).

use std::collections::HashMap;

/// Table de pricing par modèle.
#[derive(Clone, Debug)]
pub struct PricingTable {
    /// model_id → (input cents/M, output cents/M)
    rates: HashMap<String, (u32, u32)>,
}

impl PricingTable {
    /// Table Anthropic (Claude family, R1 frozen 2026-05).
    #[must_use]
    pub fn anthropic() -> Self {
        let mut rates = HashMap::new();
        // Claude Opus 4.6 — $15 in / $75 out par million tokens.
        rates.insert("claude-opus-4-6".into(), (1500, 7500));
        // Claude Sonnet 4.6 — $3 in / $15 out (DEFAULT R1a).
        rates.insert("claude-sonnet-4-6".into(), (300, 1500));
        // Claude Haiku 4.5 — $1 in / $5 out.
        rates.insert("claude-haiku-4-5".into(), (100, 500));
        rates.insert("claude-haiku-4-5-20251001".into(), (100, 500));
        Self { rates }
    }

    /// Table Google Gemini.
    #[must_use]
    pub fn gemini() -> Self {
        let mut rates = HashMap::new();
        // Gemini 2.5 Pro — $1.25 in / $5 out (sub-200k context).
        rates.insert("gemini-2.5-pro".into(), (125, 500));
        // Gemini 2.5 Flash — $0.075 in / $0.30 out (DEFAULT R1a).
        // Stocké en cents/M = 7.5 cents → arrondi 8 ; 30 cents → 30.
        // En vérité : $0.075 = 7.5 cents/M, on stocke en *0.1 cents/M
        // pour précision : (75, 300) puis div par 10 au calc.
        // Plus simple : on garde en cents/M arrondi sup pour éviter dette.
        rates.insert("gemini-2.5-flash".into(), (8, 30));
        // Gemini 2.5 Flash-lite — $0.04 in / $0.15 out.
        rates.insert("gemini-2.5-flash-lite".into(), (4, 15));
        Self { rates }
    }

    /// Calcule le coût en cents.  Si le modèle est inconnu, fallback
    /// sur la moyenne (1000, 5000) — conservateur pour budget tracker.
    #[must_use]
    pub fn calc(&self, model: &str, input_tokens: u32, output_tokens: u32) -> u32 {
        let (in_rate, out_rate) = self.rates.get(model).copied().unwrap_or((1000, 5000));
        let total = (input_tokens as u64 * in_rate as u64)
            + (output_tokens as u64 * out_rate as u64);
        // total est en (cents × tokens) → diviser par M tokens.
        // Arrondi sup pour le budget tracker (conservative).
        ((total + 999_999) / 1_000_000) as u32
    }

    /// Vérifie qu'un modèle est listé.
    #[must_use]
    pub fn has_model(&self, model: &str) -> bool {
        self.rates.contains_key(model)
    }

    /// Liste des modèles connus.
    #[must_use]
    pub fn models(&self) -> Vec<&str> {
        self.rates.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_sonnet_default_cost() {
        let p = PricingTable::anthropic();
        // 1000 input + 1000 output tokens sur Sonnet 4.6 :
        // (1000 × 300 + 1000 × 1500) / 1_000_000 = 1.8 cents → arrondi 2
        let cost = p.calc("claude-sonnet-4-6", 1000, 1000);
        assert_eq!(cost, 2);
    }

    #[test]
    fn anthropic_opus_premium_cost() {
        let p = PricingTable::anthropic();
        // 10_000 tokens chacun sur Opus :
        // (10_000 × 1500 + 10_000 × 7500) / 1_000_000 = 90 cents
        let cost = p.calc("claude-opus-4-6", 10_000, 10_000);
        assert_eq!(cost, 90);
    }

    #[test]
    fn gemini_flash_super_cheap() {
        let p = PricingTable::gemini();
        // 10_000 in + 10_000 out sur Flash :
        // (10_000 × 8 + 10_000 × 30) / 1_000_000 = 0.38 → arrondi sup 1
        let cost = p.calc("gemini-2.5-flash", 10_000, 10_000);
        assert!(cost <= 1, "Flash should be ≤ 1 cent for 20k tokens, got {cost}");
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        let p = PricingTable::anthropic();
        // Modèle inconnu → fallback (1000, 5000) cents/M (conservatif)
        let cost = p.calc("unknown-model-xyz", 1000, 1000);
        // (1000 × 1000 + 1000 × 5000) / 1_000_000 = 6
        assert_eq!(cost, 6);
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let p = PricingTable::anthropic();
        let cost = p.calc("claude-sonnet-4-6", 0, 0);
        assert_eq!(cost, 0);
    }

    #[test]
    fn has_model_lookup() {
        let a = PricingTable::anthropic();
        assert!(a.has_model("claude-sonnet-4-6"));
        assert!(!a.has_model("gemini-2.5-pro"));
        let g = PricingTable::gemini();
        assert!(g.has_model("gemini-2.5-flash"));
        assert!(!g.has_model("claude-opus-4-6"));
    }

    #[test]
    fn models_listing_non_empty() {
        let a = PricingTable::anthropic();
        assert!(!a.models().is_empty());
        let g = PricingTable::gemini();
        assert!(!g.models().is_empty());
    }
}
