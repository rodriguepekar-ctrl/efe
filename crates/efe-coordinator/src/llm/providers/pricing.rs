// SPDX-License-Identifier: UNLICENSED
//! `PricingTable` — calcul du coût en cents et en microcents (ADR-P3R1-004 + R3.S1).
//!
//! Source : pricing publié par Anthropic, Google et OpenAI (vérifié au moment du
//! commit R1 — pricing évolue, à reverifier au déploiement).
//!
//! ## R3.S1 (microcents)
//! - Stockage primaire : `microrates: HashMap<model, (u64, u64)>` en
//!   microcents/M tokens (1 microcent = 1/1000 cent = 1/100 000 dollar).
//! - Compatibilité : `rates: HashMap<model, (u32, u32)>` en cents/M (legacy)
//!   reste exposé via `calc()`; `calc_microcents()` est la nouvelle voie
//!   précise (Gemini Flash : exact 7_500 µ¢/M, fini l'arrondi à 8).

use std::collections::HashMap;

/// Table de pricing par modèle (cents/M, legacy + microcents/M, R3.S1).
#[derive(Clone, Debug)]
pub struct PricingTable {
    /// model_id → (input cents/M, output cents/M) — LEGACY u32, conservé
    /// pour `calc()` rétrocompatible.
    rates: HashMap<String, (u32, u32)>,
    /// model_id → (input µ¢/M, output µ¢/M) — R3.S1, précision sub-cent.
    /// 1 µ¢ = 1/1000 cent = 1/100 000 USD ⇒ Gemini Flash $0.075/M = 7_500 µ¢/M.
    microrates: HashMap<String, (u64, u64)>,
}

impl PricingTable {
    /// Table Anthropic (Claude family, R1 frozen 2026-05).
    ///
    /// R3.S1 : ajoute la table microrates en parallèle. Les valeurs sont
    /// exactes (× 1000 par rapport aux cents/M car aucun prix Claude ne
    /// présente de sous-cent en R1/R2).
    #[must_use]
    pub fn anthropic() -> Self {
        let mut rates = HashMap::new();
        let mut microrates = HashMap::new();
        // Claude Opus 4.6 — $15 in / $75 out par million tokens.
        rates.insert("claude-opus-4-6".into(), (1500, 7500));
        microrates.insert("claude-opus-4-6".into(), (1_500_000_u64, 7_500_000_u64));
        // Claude Sonnet 4.6 — $3 in / $15 out (DEFAULT R1a).
        rates.insert("claude-sonnet-4-6".into(), (300, 1500));
        microrates.insert("claude-sonnet-4-6".into(), (300_000_u64, 1_500_000_u64));
        // Claude Haiku 4.5 — $1 in / $5 out.
        rates.insert("claude-haiku-4-5".into(), (100, 500));
        microrates.insert("claude-haiku-4-5".into(), (100_000_u64, 500_000_u64));
        rates.insert("claude-haiku-4-5-20251001".into(), (100, 500));
        microrates.insert(
            "claude-haiku-4-5-20251001".into(),
            (100_000_u64, 500_000_u64),
        );
        Self { rates, microrates }
    }

    /// Table OpenAI GPT.
    ///
    /// Pricing officiel (vérifié au moment du commit R1b — à reverifier au
    /// déploiement, OpenAI met à jour fréquemment).
    /// Valeurs en cents par M tokens (entiers, arrondi sup) + microcents R3.S1.
    #[must_use]
    pub fn openai() -> Self {
        let mut rates = HashMap::new();
        let mut microrates = HashMap::new();
        // GPT-5 — $5 in / $15 out (estimation 2026 prélancement).
        rates.insert("gpt-5".into(), (500, 1500));
        microrates.insert("gpt-5".into(), (500_000, 1_500_000));
        rates.insert("gpt-5-mini".into(), (50, 200));
        microrates.insert("gpt-5-mini".into(), (50_000, 200_000));
        // GPT-4o — $2.50 in / $10 out (DEFAULT R1b).
        rates.insert("gpt-4o".into(), (250, 1000));
        microrates.insert("gpt-4o".into(), (250_000, 1_000_000));
        rates.insert("gpt-4o-2024-11-20".into(), (250, 1000));
        microrates.insert("gpt-4o-2024-11-20".into(), (250_000, 1_000_000));
        // GPT-4o-mini — $0.15 in / $0.60 out (15 cents/M = exact 150_000 µ¢/M).
        rates.insert("gpt-4o-mini".into(), (15, 60));
        microrates.insert("gpt-4o-mini".into(), (15_000, 60_000));
        // GPT-4-turbo — $10 in / $30 out.
        rates.insert("gpt-4-turbo".into(), (1000, 3000));
        microrates.insert("gpt-4-turbo".into(), (1_000_000, 3_000_000));
        rates.insert("gpt-4".into(), (3000, 6000));
        microrates.insert("gpt-4".into(), (3_000_000, 6_000_000));
        // GPT-3.5-turbo — $0.50 in / $1.50 out.
        rates.insert("gpt-3.5-turbo".into(), (50, 150));
        microrates.insert("gpt-3.5-turbo".into(), (50_000, 150_000));
        // o1 / o3 reasoning models — $15 in / $60 out.
        rates.insert("o1".into(), (1500, 6000));
        microrates.insert("o1".into(), (1_500_000, 6_000_000));
        // o3-mini — $1.10 in / $4.40 out.
        rates.insert("o3-mini".into(), (110, 440));
        microrates.insert("o3-mini".into(), (110_000, 440_000));
        Self { rates, microrates }
    }

    /// Table Google Gemini.
    ///
    /// R3.S1 (D7) : la dette historique sur Gemini Flash (8 cents/M au lieu
    /// de 7.5 → +6.7% d'erreur) est résolue dans `microrates` : on stocke
    /// l'exact `7_500` µ¢/M. La table `rates` cents/M reste arrondie pour
    /// compat legacy ; les nouveaux callers doivent passer par
    /// `calc_microcents()`.
    #[must_use]
    pub fn gemini() -> Self {
        let mut rates = HashMap::new();
        let mut microrates = HashMap::new();
        // Gemini 2.5 Pro — $1.25 in / $5 out (sub-200k context).
        rates.insert("gemini-2.5-pro".into(), (125, 500));
        microrates.insert("gemini-2.5-pro".into(), (125_000, 500_000));
        // Gemini 2.5 Flash — $0.075 in / $0.30 out (DEFAULT R1a).
        // legacy : (8, 30) avec arrondi sup → +6.7% input.
        // R3.S1 : EXACT (7_500, 30_000) µ¢/M.
        rates.insert("gemini-2.5-flash".into(), (8, 30));
        microrates.insert("gemini-2.5-flash".into(), (7_500, 30_000));
        // Gemini 2.5 Flash-lite — $0.04 in / $0.15 out → (4_000, 15_000) µ¢/M exact.
        rates.insert("gemini-2.5-flash-lite".into(), (4, 15));
        microrates.insert("gemini-2.5-flash-lite".into(), (4_000, 15_000));
        Self { rates, microrates }
    }

    /// **R3.S4** — Table SLM **local** (llama-server, ex: Gemma 2 1.5B).
    ///
    /// Coût = énergie amortie (NON zéro — honnêteté). Défaut ~1500 µ¢/M
    /// (~1110 énergie @ $0.15/kWh, 75 tok/s, 20 W + ~390 amortissement
    /// hardware). Calibrable via RAPL (bench framework §38). C'est un
    /// **hint** pour le router Mission A/B ; en Speculative, le
    /// `LocalSlmProvider` self-reporte son coût (qui prime).
    #[must_use]
    pub fn local() -> Self {
        let mut rates = HashMap::new();
        let mut microrates = HashMap::new();
        // Gemma 2 1.5B — énergie amortie ~1500 µ¢/M (≈ 0.15 ¢/M).
        // cents/M arrondi sup : 1500 µ¢ → 2 cents/M (legacy conservateur).
        rates.insert("gemma-2-1.5b".into(), (2, 2));
        microrates.insert("gemma-2-1.5b".into(), (1_500, 1_500));
        // Qwen 0.5B — moins cher en énergie (plus rapide) mais plus bruité.
        rates.insert("qwen-2.5-0.5b".into(), (1, 1));
        microrates.insert("qwen-2.5-0.5b".into(), (800, 800));
        // Llama 3.2 3B — plus lent / plus d'énergie.
        rates.insert("llama-3.2-3b".into(), (3, 3));
        microrates.insert("llama-3.2-3b".into(), (3_000, 3_000));
        Self { rates, microrates }
    }

    /// Calcule le coût en cents.  Si le modèle est inconnu, fallback
    /// sur la moyenne (1000, 5000) — conservateur pour budget tracker.
    ///
    /// **LEGACY** : utiliser `calc_microcents()` en R3+ pour la précision.
    #[must_use]
    pub fn calc(&self, model: &str, input_tokens: u32, output_tokens: u32) -> u32 {
        let (in_rate, out_rate) = self.rates.get(model).copied().unwrap_or((1000, 5000));
        let total = (input_tokens as u64 * in_rate as u64)
            + (output_tokens as u64 * out_rate as u64);
        // total est en (cents × tokens) → diviser par M tokens.
        // Arrondi sup pour le budget tracker (conservative).
        ((total + 999_999) / 1_000_000) as u32
    }

    /// **R3.S1** : Calcule le coût en **microcents** (1 µ¢ = 1/1000 cent).
    ///
    /// Précision sub-cent : Gemini Flash sur 1k tokens = 7.5 µ¢ + 30 µ¢ = 37 µ¢
    /// au lieu d'1 cent arrondi (perte de 73% d'info en cents).
    ///
    /// Si le modèle est inconnu, fallback sur `(1_000_000, 5_000_000)` µ¢/M
    /// (= legacy `(1000, 5000)` cents/M × 1000) — même politique conservative.
    ///
    /// Formule : `floor((tokens × µ¢/M) / 1_000_000)` (pas d'arrondi sup ici
    /// — la précision µ¢ rend l'arrondi inutile, on retient la donnée brute).
    #[must_use]
    pub fn calc_microcents(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> u64 {
        let (in_rate, out_rate) = self
            .microrates
            .get(model)
            .copied()
            .unwrap_or((1_000_000_u64, 5_000_000_u64));
        let in_mc =
            (input_tokens as u64).saturating_mul(in_rate) / 1_000_000;
        let out_mc =
            (output_tokens as u64).saturating_mul(out_rate) / 1_000_000;
        in_mc.saturating_add(out_mc)
    }

    /// **R3.S1** : retourne `(input µ¢/M, output µ¢/M)` pour un modèle, ou
    /// `None` si inconnu.  Utilisé par `LlmMetadata` pour propager le pricing
    /// jusqu'au scoring table.
    #[must_use]
    pub fn rates_microcents(&self, model: &str) -> Option<(u64, u64)> {
        self.microrates.get(model).copied()
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
    fn openai_gpt4o_default_cost() {
        let p = PricingTable::openai();
        // 1000 in + 1000 out sur GPT-4o : (1000 × 250 + 1000 × 1000) / 1M = 1.25 → 2
        let cost = p.calc("gpt-4o", 1000, 1000);
        assert_eq!(cost, 2);
    }

    #[test]
    fn openai_gpt4o_mini_cheap() {
        let p = PricingTable::openai();
        // GPT-4o-mini : (1000 × 15 + 1000 × 60) / 1M = 0.075 → 1 (arrondi sup)
        let cost = p.calc("gpt-4o-mini", 1000, 1000);
        assert_eq!(cost, 1);
    }

    #[test]
    fn openai_o1_reasoning_premium() {
        let p = PricingTable::openai();
        // o1 reasoning : (10_000 × 1500 + 10_000 × 6000) / 1M = 75 cents
        let cost = p.calc("o1", 10_000, 10_000);
        assert_eq!(cost, 75);
    }

    #[test]
    fn openai_models_listing() {
        let p = PricingTable::openai();
        let m = p.models();
        assert!(m.iter().any(|n| *n == "gpt-4o"));
        assert!(m.iter().any(|n| *n == "gpt-4o-mini"));
        assert!(m.iter().any(|n| *n == "gpt-5"));
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

    // ============================================================
    // R3.S1 — Tests microcents
    // ============================================================

    #[test]
    fn r3s1_pricing_calc_microcents_claude_sonnet_1k() {
        let p = PricingTable::anthropic();
        // 1000 in + 1000 out @ Sonnet (300_000 µ¢/M in, 1_500_000 µ¢/M out)
        // = (1000 × 300_000) / 1M + (1000 × 1_500_000) / 1M
        // = 300 + 1_500 = 1_800 µ¢ = 1.8 cents EXACT.
        let mc = p.calc_microcents("claude-sonnet-4-6", 1_000, 1_000);
        assert_eq!(mc, 1_800);
    }

    #[test]
    fn r3s1_pricing_calc_microcents_gemini_flash_exact_no_rounding() {
        let p = PricingTable::gemini();
        // R3.S1 D7 fix : 1000 in + 1000 out @ Flash
        // microrates = (7_500, 30_000) µ¢/M
        // = (1000 × 7_500) / 1M + (1000 × 30_000) / 1M
        // = 7 + 30 = 37 µ¢ (avant R3.S1, legacy : 0 cent !).
        let mc = p.calc_microcents("gemini-2.5-flash", 1_000, 1_000);
        assert_eq!(mc, 37, "Gemini Flash 1k/1k tokens doit être 37 µ¢ exact");
    }

    #[test]
    fn r3s1_pricing_calc_microcents_gemini_flash_10k() {
        let p = PricingTable::gemini();
        // 10_000 in + 10_000 out @ Flash
        // = (10_000 × 7_500) / 1M + (10_000 × 30_000) / 1M
        // = 75 + 300 = 375 µ¢ (= 0.375 cent).
        let mc = p.calc_microcents("gemini-2.5-flash", 10_000, 10_000);
        assert_eq!(mc, 375);
    }

    #[test]
    fn r3s1_pricing_calc_microcents_unknown_model_falls_back() {
        let p = PricingTable::anthropic();
        // Modèle inconnu → fallback (1_000_000, 5_000_000) µ¢/M
        // 1000 in + 1000 out = (1000 × 1_000_000)/1M + (1000 × 5_000_000)/1M
        // = 1_000 + 5_000 = 6_000 µ¢.
        let mc = p.calc_microcents("unknown-model-xyz", 1_000, 1_000);
        assert_eq!(mc, 6_000);
    }

    #[test]
    fn r3s1_pricing_calc_microcents_zero_tokens_zero_cost() {
        let p = PricingTable::anthropic();
        let mc = p.calc_microcents("claude-sonnet-4-6", 0, 0);
        assert_eq!(mc, 0);
    }

    #[test]
    fn r3s1_pricing_rates_microcents_returns_exact_values() {
        let g = PricingTable::gemini();
        let r = g.rates_microcents("gemini-2.5-flash");
        assert_eq!(r, Some((7_500, 30_000)));
        let p = PricingTable::anthropic();
        let r = p.rates_microcents("claude-sonnet-4-6");
        assert_eq!(r, Some((300_000, 1_500_000)));
        // Inconnu → None.
        assert!(p.rates_microcents("gpt-4o").is_none());
    }

    #[test]
    fn r3s1_pricing_calc_microcents_vs_calc_cents_consistency() {
        // Sur Claude (pas de sous-cent), `calc_microcents() / 1000` doit
        // être ≤ `calc()` (calc arrondit sup).
        let p = PricingTable::anthropic();
        let mc = p.calc_microcents("claude-sonnet-4-6", 1_000, 1_000);
        let c = p.calc("claude-sonnet-4-6", 1_000, 1_000);
        // mc = 1_800, c = 2 (1.8 arrondi sup).
        let mc_floor = mc / 1000;
        assert!(mc_floor <= c as u64);
    }

    #[test]
    fn r3s1_pricing_calc_microcents_saturating_does_not_overflow() {
        // Token counts géants ne doivent pas paniquer (saturating math).
        let p = PricingTable::anthropic();
        let mc = p.calc_microcents("claude-opus-4-6", u32::MAX, u32::MAX);
        // On vérifie juste qu'on n'a pas paniqué et qu'on a un nombre > 0.
        assert!(mc > 0);
    }

    #[test]
    fn r3s1_pricing_gemini_flash_legacy_cents_still_works() {
        // calc() (legacy) doit toujours produire son arrondi sup historique.
        let p = PricingTable::gemini();
        // 1000 in + 1000 out @ Flash (8, 30) cents/M
        // = (1000 × 8 + 1000 × 30) / 1M = 0.038 → arrondi sup 1 cent.
        let c = p.calc("gemini-2.5-flash", 1_000, 1_000);
        assert_eq!(c, 1);
    }

    #[test]
    fn r3s1_pricing_openai_gpt4o_mini_exact() {
        let p = PricingTable::openai();
        // GPT-4o-mini : (15_000, 60_000) µ¢/M
        // 1000 in + 1000 out = 15 + 60 = 75 µ¢ (= 0.075 cent EXACT).
        let mc = p.calc_microcents("gpt-4o-mini", 1_000, 1_000);
        assert_eq!(mc, 75);
    }

    // ============================================================
    // R3.S1 — Tests architecture-spec'd §4.2 (Gap I)
    // ============================================================

    #[test]
    fn r3s1_pricing_table_claude_microcents_matches_cents_x1000() {
        // §4.2 architecture spec : Claude n'a pas de sous-cent → la table
        // µ¢/M doit être strictement cents/M × 1000.
        let p = PricingTable::anthropic();
        for model in &[
            "claude-sonnet-4-6",
            "claude-opus-4-6",
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
        ] {
            let (in_mc, out_mc) = p.rates_microcents(model).unwrap_or_else(|| {
                panic!("Claude model {model} doit avoir une entrée microrates")
            });
            // Calcul cross-référence sur 1M tokens (vs calc() legacy cents/M).
            let in_cents_legacy = p.calc(model, 1_000_000, 0);
            let out_cents_legacy = p.calc(model, 0, 1_000_000);
            // in_mc = in_cents_legacy × 1000 (Claude pas de sous-cent).
            assert_eq!(
                in_mc,
                (in_cents_legacy as u64) * 1000,
                "Claude {model} in_mc/M doit être cents/M × 1000 (= {})",
                in_cents_legacy
            );
            assert_eq!(
                out_mc,
                (out_cents_legacy as u64) * 1000,
                "Claude {model} out_mc/M doit être cents/M × 1000 (= {})",
                out_cents_legacy
            );
        }
    }

    #[test]
    fn r3s1_pricing_table_gemini_flash_microcents_exact() {
        // §4.2 architecture spec : Gemini Flash 7_500 µ¢/M EXACT (vs 8 cents/M
        // arrondi en legacy = +6.7% drift).
        let p = PricingTable::gemini();
        let (in_mc, out_mc) = p.rates_microcents("gemini-2.5-flash").unwrap();
        assert_eq!(in_mc, 7_500, "Gemini Flash input µ¢/M doit être EXACT 7_500");
        assert_eq!(out_mc, 30_000, "Gemini Flash output µ¢/M doit être 30_000");
        // Et le legacy cents/M reste 8/30 (rétrocompat).
        let cents_in = p.calc("gemini-2.5-flash", 1_000_000, 0);
        assert_eq!(cents_in, 8, "Gemini Flash legacy cents/M reste 8 (arrondi sup)");
    }

    #[test]
    fn r3s1_pricing_cost_microcents_gemini_fixture_01() {
        // §4.2 : fixture 01 avec 50 input + 20 output tokens, Gemini Flash :
        // (50 × 7_500) / 1M + (20 × 30_000) / 1M = 0 + 0 = 0 µ¢
        // Edge case integer division : 0.375 + 0.6 = 0.975 µ¢ réel → tronqué.
        // Précision µ¢ ATTEINT SA LIMITE sur très petites fixtures, mais
        // c'est mieux que cents (qui s'arrondit à 1 ¢ = 1_000 µ¢).
        let p = PricingTable::gemini();
        let mc = p.calc_microcents("gemini-2.5-flash", 50, 20);
        // Architecture spec note : "Précision limitée mais MIEUX que u32 cents".
        assert_eq!(mc, 0, "Gemini fixture 01 (50/20 tokens) → 0 µ¢ (limite intégrale)");
    }

    #[test]
    fn r3s4_pricing_local_gemma_far_below_gemini() {
        // 1000 in + 1000 out @ Gemma local (1_500/1_500 µ¢/M) = 1.5 + 1.5 = 3 µ¢.
        let local = PricingTable::local();
        let mc_local = local.calc_microcents("gemma-2-1.5b", 1_000, 1_000);
        // 1000 in @ 1500 µ¢/M = 1 (floor) ; idem out → 2 µ¢.
        assert_eq!(mc_local, 2);
        // Gemini Flash : 1000/1000 = 37 µ¢ → local ~12× moins cher.
        let gemini = PricingTable::gemini();
        let mc_gemini = gemini.calc_microcents("gemini-2.5-flash", 1_000, 1_000);
        assert!(
            mc_local * 10 < mc_gemini,
            "local ({mc_local} µ¢) doit être ≪ gemini ({mc_gemini} µ¢)"
        );
    }

    #[test]
    fn r3s4_pricing_local_known_models() {
        let local = PricingTable::local();
        assert!(local.has_model("gemma-2-1.5b"));
        assert!(local.has_model("qwen-2.5-0.5b"));
        assert!(local.has_model("llama-3.2-3b"));
        // µ¢ exacts.
        assert_eq!(local.rates_microcents("gemma-2-1.5b"), Some((1_500, 1_500)));
    }
}
