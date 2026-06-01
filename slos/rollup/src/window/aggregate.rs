//! PR #22 § 8.2 — aggregation rules T1 → T2.
//!
//! For **ratio-of-sums** SLOs (SLO-1, SLO-3) :
//! ```text
//!   SLI^30d = (Σ N_i) / (Σ D_i)
//! ```
//!
//! For **window-compliance** SLOs (SLO-2, SLO-4) :
//! ```text
//!   SLI^30d = mean(C_i) over windows where C_i is defined (not NaN)
//! ```
//!
//! These are different formulas ; mixing them is a contract violation
//! (tested via property test in `tests/window_property.rs`).

use crate::sli::{SliPoint, SliVerdict};
use crate::SliAggregation;

/// Aggregate a slice of [`SliPoint`]s into a single rolled-up value.
///
/// `agg` chooses the formula.  Returns `(value, numerator_sum,
/// denominator_sum, verdict)`.
#[must_use]
pub fn aggregate(points: &[SliPoint], agg: SliAggregation, target: f64) -> AggregatedSli {
    match agg {
        SliAggregation::RatioOfSums => {
            let mut num = 0u64;
            let mut den = 0u64;
            for p in points {
                if matches!(p.verdict, SliVerdict::Indeterminate) {
                    continue;
                }
                num += p.numerator;
                den += p.denominator;
            }
            if den == 0 {
                return AggregatedSli {
                    value: f64::NAN,
                    numerator: 0,
                    denominator: 0,
                    valid_count: 0,
                    verdict: SliVerdict::Indeterminate,
                };
            }
            let v = num as f64 / den as f64;
            let verdict = if v >= target { SliVerdict::Pass } else { SliVerdict::Fail };
            AggregatedSli {
                value: v,
                numerator: num,
                denominator: den,
                valid_count: points.iter().filter(|p| !matches!(p.verdict, SliVerdict::Indeterminate)).count() as u64,
                verdict,
            }
        }
        SliAggregation::WindowCompliance => {
            let bits: Vec<u8> = points
                .iter()
                .filter(|p| p.compliance_bit != 255)
                .map(|p| p.compliance_bit)
                .collect();
            if bits.is_empty() {
                return AggregatedSli {
                    value: f64::NAN,
                    numerator: 0,
                    denominator: 0,
                    valid_count: 0,
                    verdict: SliVerdict::Indeterminate,
                };
            }
            let sum: u64 = bits.iter().map(|&b| b as u64).sum();
            let v = sum as f64 / bits.len() as f64;
            let verdict = if v >= target { SliVerdict::Pass } else { SliVerdict::Fail };
            AggregatedSli {
                value: v,
                numerator: sum,
                denominator: bits.len() as u64,
                valid_count: bits.len() as u64,
                verdict,
            }
        }
    }
}

/// Result of [`aggregate`].
#[derive(Debug, Clone)]
pub struct AggregatedSli {
    pub value: f64,
    pub numerator: u64,
    pub denominator: u64,
    pub valid_count: u64,
    pub verdict: SliVerdict,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sli::SliVerdict;
    use crate::window::WindowSpec;
    use crate::SloId;

    fn pt_ratio(num: u64, den: u64, verdict: SliVerdict) -> SliPoint {
        SliPoint {
            slo_id: SloId::L2Availability,
            window: WindowSpec::FiveMin,
            window_start_unix: 0,
            value: if den > 0 { num as f64 / den as f64 } else { f64::NAN },
            numerator: num,
            denominator: den,
            compliance_bit: 255,
            coverage_pct: 1.0,
            source: "prod".into(),
            region: "global".into(),
            verdict,
        }
    }

    fn pt_bit(bit: u8) -> SliPoint {
        SliPoint {
            slo_id: SloId::L2LatencyP95,
            window: WindowSpec::FiveMin,
            window_start_unix: 0,
            value: 0.0,
            numerator: bit as u64,
            denominator: 1,
            compliance_bit: bit,
            coverage_pct: 1.0,
            source: "prod".into(),
            region: "global".into(),
            verdict: if bit == 1 { SliVerdict::Pass } else if bit == 0 { SliVerdict::Fail } else { SliVerdict::Indeterminate },
        }
    }

    #[test]
    fn ratio_aggregation_sums_then_divides() {
        let pts = vec![
            pt_ratio(99, 100, SliVerdict::Fail),    // 0.99
            pt_ratio(100, 100, SliVerdict::Pass),   // 1.0
            pt_ratio(50, 100, SliVerdict::Fail),    // 0.50
        ];
        // sum = 249/300 = 0.83
        let agg = aggregate(&pts, SliAggregation::RatioOfSums, 0.995);
        assert_eq!(agg.numerator, 249);
        assert_eq!(agg.denominator, 300);
        assert!((agg.value - 0.83).abs() < 1e-9);
        assert_eq!(agg.verdict, SliVerdict::Fail);
    }

    #[test]
    fn ratio_aggregation_skips_indeterminate() {
        let pts = vec![
            pt_ratio(0, 0, SliVerdict::Indeterminate),
            pt_ratio(995, 1000, SliVerdict::Pass),
        ];
        let agg = aggregate(&pts, SliAggregation::RatioOfSums, 0.995);
        assert_eq!(agg.numerator, 995);
        assert_eq!(agg.denominator, 1000);
        assert_eq!(agg.verdict, SliVerdict::Pass);
    }

    #[test]
    fn ratio_all_indeterminate_yields_indeterminate() {
        let pts = vec![pt_ratio(0, 0, SliVerdict::Indeterminate); 5];
        let agg = aggregate(&pts, SliAggregation::RatioOfSums, 0.995);
        assert_eq!(agg.verdict, SliVerdict::Indeterminate);
        assert!(agg.value.is_nan());
    }

    #[test]
    fn window_compliance_mean_of_bits() {
        // 8/10 = 0.80
        let pts: Vec<_> = (0..10).map(|i| pt_bit(if i < 8 { 1 } else { 0 })).collect();
        let agg = aggregate(&pts, SliAggregation::WindowCompliance, 0.99);
        assert!((agg.value - 0.80).abs() < 1e-9);
        assert_eq!(agg.verdict, SliVerdict::Fail);
    }

    #[test]
    fn window_compliance_excludes_nan_bits() {
        let pts = vec![pt_bit(1), pt_bit(1), pt_bit(255), pt_bit(255)];
        let agg = aggregate(&pts, SliAggregation::WindowCompliance, 0.99);
        // 2/2 = 1.0 (the 2 NaN bits are excluded)
        assert!((agg.value - 1.0).abs() < 1e-9);
        assert_eq!(agg.denominator, 2);
        assert_eq!(agg.verdict, SliVerdict::Pass);
    }

    #[test]
    fn window_compliance_all_nan_indeterminate() {
        let pts = vec![pt_bit(255), pt_bit(255)];
        let agg = aggregate(&pts, SliAggregation::WindowCompliance, 0.99);
        assert_eq!(agg.verdict, SliVerdict::Indeterminate);
    }

    #[test]
    fn empty_input_yields_indeterminate() {
        let agg = aggregate(&[], SliAggregation::RatioOfSums, 0.995);
        assert_eq!(agg.verdict, SliVerdict::Indeterminate);
        let agg = aggregate(&[], SliAggregation::WindowCompliance, 0.99);
        assert_eq!(agg.verdict, SliVerdict::Indeterminate);
    }
}
