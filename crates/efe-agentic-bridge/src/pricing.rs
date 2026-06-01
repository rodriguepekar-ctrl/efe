// SPDX-License-Identifier: UNLICENSED
//! **Sprint 18 — N26 + ADR-4 + G5** : Anthropic pricing table +
//! cost-microcents computation helpers.
//!
//! The architecture (annex § 14) defers a real-time pricing
//! fetcher to Sprint 19 N27.  Sprint 18 ships a **baked snapshot**
//! of Anthropic's published per-million-token rates for the three
//! models we support.  Unknown models fall back to the most
//! common rate (Sonnet 4.6) so cost emission never panics.
//!
//! ## Units
//!
//! - Per-model rates are stored as `microcents per 1k tokens`
//!   (a `microcent` = 1/10_000 of a cent ; the unit used across
//!   N17 `CostAccounting` and N21 `cost_microcents`).
//! - 1 cent = 10_000 microcents.
//! - 1 USD = 100 cents = 1_000_000 microcents.
//! - Anthropic's published prices (as of 2026-05-22) :
//!   - `claude-sonnet-4-6` : input $3 / Mtoken, output $15 / Mtoken
//!     → 3_000 mc / 1k input, 15_000 mc / 1k output.
//!   - `claude-haiku-3-5` : input $0.80 / Mtoken, output $4 / Mtoken
//!     → 800 mc / 1k input, 4_000 mc / 1k output.
//!   - `claude-opus-4` : input $15 / Mtoken, output $75 / Mtoken
//!     → 15_000 mc / 1k input, 75_000 mc / 1k output.
//!
//! ## Versioning
//!
//! [`PRICES_TABLE_VERSION`] is bumped on every table change.  The
//! version is stamped on the emitted `CostAccounting` so a future
//! reconciliation job can recompute historical costs using the
//! exact table that was active at call time.

use std::collections::HashMap;
use std::sync::OnceLock;

/// **N26 + ADR-4** : current pricing table version.  Bump on
/// every modification to the rates below.
pub const PRICES_TABLE_VERSION: u32 = 1;

/// **N26** : per-model pricing in microcents per 1k tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPricing {
    /// Input cost in microcents per 1k tokens.
    pub input_microcents_per_1k: i64,
    /// Output cost in microcents per 1k tokens.
    pub output_microcents_per_1k: i64,
}

/// **N26** : fallback pricing when the model is not in the table.
/// Equals Sonnet 4.6 (the most common pick).  Ensures cost
/// emission never panics on a new / mistyped model id.
pub const FALLBACK_PRICING: ModelPricing = ModelPricing {
    input_microcents_per_1k: 3_000,
    output_microcents_per_1k: 15_000,
};

/// **N26 + ADR-4 + G5** : lookup pricing for an Anthropic model.
/// Returns [`FALLBACK_PRICING`] for unknown models.
pub fn anthropic_pricing_for(model: &str) -> ModelPricing {
    static TABLE: OnceLock<HashMap<&'static str, ModelPricing>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        // Sonnet 4.6 (default).
        m.insert(
            "claude-sonnet-4-6",
            ModelPricing {
                input_microcents_per_1k: 3_000,
                output_microcents_per_1k: 15_000,
            },
        );
        // Haiku 3.5 (low-cost).
        m.insert(
            "claude-haiku-3-5",
            ModelPricing {
                input_microcents_per_1k: 800,
                output_microcents_per_1k: 4_000,
            },
        );
        // Opus 4 (premium).
        m.insert(
            "claude-opus-4",
            ModelPricing {
                input_microcents_per_1k: 15_000,
                output_microcents_per_1k: 75_000,
            },
        );
        m
    });
    t.get(model).copied().unwrap_or(FALLBACK_PRICING)
}

/// **N26 + G5** : compute the total cost in microcents from a
/// per-model pricing + token counts.  Uses saturating arithmetic
/// to keep the result within `i64` even on adversarial inputs.
pub fn compute_cost_microcents(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> i64 {
    let p = anthropic_pricing_for(model);
    let input_cost = (input_tokens as i64)
        .saturating_mul(p.input_microcents_per_1k)
        .saturating_div(1_000);
    let output_cost = (output_tokens as i64)
        .saturating_mul(p.output_microcents_per_1k)
        .saturating_div(1_000);
    input_cost.saturating_add(output_cost)
}

/// **N26 + G5** : the same computation but returning a `f64`
/// cost in cents — the unit used by N17 `CostAccounting.cost_cents`.
pub fn compute_cost_cents(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    (compute_cost_microcents(model, input_tokens, output_tokens) as f64) / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sonnet_pricing_matches_published_rate() {
        let p = anthropic_pricing_for("claude-sonnet-4-6");
        // $3 / Mtoken input = 3_000 mc / 1k tokens.
        assert_eq!(p.input_microcents_per_1k, 3_000);
        // $15 / Mtoken output = 15_000 mc / 1k tokens.
        assert_eq!(p.output_microcents_per_1k, 15_000);
    }

    #[test]
    fn haiku_pricing_matches_published_rate() {
        let p = anthropic_pricing_for("claude-haiku-3-5");
        assert_eq!(p.input_microcents_per_1k, 800);
        assert_eq!(p.output_microcents_per_1k, 4_000);
    }

    #[test]
    fn opus_pricing_matches_published_rate() {
        let p = anthropic_pricing_for("claude-opus-4");
        assert_eq!(p.input_microcents_per_1k, 15_000);
        assert_eq!(p.output_microcents_per_1k, 75_000);
    }

    #[test]
    fn unknown_model_returns_fallback() {
        let p = anthropic_pricing_for("not-a-real-model");
        assert_eq!(p, FALLBACK_PRICING);
    }

    #[test]
    fn empty_model_string_returns_fallback() {
        let p = anthropic_pricing_for("");
        assert_eq!(p, FALLBACK_PRICING);
    }

    #[test]
    fn compute_cost_microcents_sonnet_1k_input_1k_output() {
        // 1k input × 3_000 / 1k = 3_000 mc ; 1k output × 15_000 / 1k = 15_000 mc.
        let total = compute_cost_microcents("claude-sonnet-4-6", 1_000, 1_000);
        assert_eq!(total, 3_000 + 15_000);
    }

    #[test]
    fn compute_cost_microcents_zero_tokens_returns_zero() {
        assert_eq!(compute_cost_microcents("claude-sonnet-4-6", 0, 0), 0);
    }

    #[test]
    fn compute_cost_microcents_input_only() {
        // 2_500 input × 3_000 / 1k = 7_500 mc.
        assert_eq!(
            compute_cost_microcents("claude-sonnet-4-6", 2_500, 0),
            7_500
        );
    }

    #[test]
    fn compute_cost_microcents_output_only() {
        // 400 output × 15_000 / 1k = 6_000 mc.
        assert_eq!(
            compute_cost_microcents("claude-sonnet-4-6", 0, 400),
            6_000
        );
    }

    #[test]
    fn compute_cost_microcents_haiku_is_cheaper_than_sonnet() {
        let sonnet = compute_cost_microcents("claude-sonnet-4-6", 1_000, 1_000);
        let haiku = compute_cost_microcents("claude-haiku-3-5", 1_000, 1_000);
        assert!(haiku < sonnet);
        // Haiku = 800 + 4_000 = 4_800 mc, Sonnet = 3_000 + 15_000 = 18_000 mc.
        assert_eq!(haiku, 4_800);
        assert_eq!(sonnet, 18_000);
    }

    #[test]
    fn compute_cost_microcents_opus_is_more_expensive_than_sonnet() {
        let sonnet = compute_cost_microcents("claude-sonnet-4-6", 1_000, 1_000);
        let opus = compute_cost_microcents("claude-opus-4", 1_000, 1_000);
        assert!(opus > sonnet);
        // Opus = 15_000 + 75_000 = 90_000 mc.
        assert_eq!(opus, 90_000);
    }

    #[test]
    fn compute_cost_microcents_saturates_on_overflow() {
        // u32::MAX × i64::MAX would overflow ; saturating_mul keeps
        // the result within i64 instead of panicking.
        let _ = compute_cost_microcents("claude-opus-4", u32::MAX, u32::MAX);
        // Should not panic ; result is at most i64::MAX.
    }

    #[test]
    fn compute_cost_cents_matches_microcents_divided_by_10000() {
        let mc = compute_cost_microcents("claude-sonnet-4-6", 1_000, 1_000);
        let c = compute_cost_cents("claude-sonnet-4-6", 1_000, 1_000);
        assert!(((mc as f64 / 10_000.0) - c).abs() < f64::EPSILON);
    }

    #[test]
    fn pricing_table_version_is_one_at_sprint_18() {
        assert_eq!(PRICES_TABLE_VERSION, 1);
    }

    #[test]
    fn anthropic_pricing_for_is_idempotent() {
        // OnceLock means repeated calls return the same table.
        let p1 = anthropic_pricing_for("claude-sonnet-4-6");
        let p2 = anthropic_pricing_for("claude-sonnet-4-6");
        assert_eq!(p1, p2);
    }
}
