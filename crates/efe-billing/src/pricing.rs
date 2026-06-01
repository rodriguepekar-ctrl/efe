// SPDX-License-Identifier: UNLICENSED
//! M31 — Pricing plan catalogue + cost computation.
//!
//! The catalogue is loaded from PG (`pricing_plan` + `pricing_plan_meter`)
//! at process start + refreshed every 5 min. Lookups are O(1) per
//! (plan, meter).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use efe_metering::Meter;

use crate::error::{BillingError, Result};

/// Cents (USD by default ; multi-currency is M31.1 scope).
pub type PriceCents = u64;

/// Unit price for a meter under a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitPrice {
    /// Free allowance per billing period. Charged at 0.
    pub free_units: u64,
    /// Price per unit in cents AFTER the free allowance.
    pub price_cents_per_unit: PriceCents,
}

impl UnitPrice {
    /// Zero-priced unit (used for the `free` plan).
    pub const ZERO: UnitPrice = UnitPrice {
        free_units: 0,
        price_cents_per_unit: 0,
    };

    /// Compute the cost in cents for the given usage volume.
    /// `usage_units = 0` always returns 0.
    #[must_use]
    pub fn cost(&self, usage_units: u64) -> PriceCents {
        if usage_units <= self.free_units {
            return 0;
        }
        let billable = usage_units - self.free_units;
        billable
            .saturating_mul(self.price_cents_per_unit)
    }
}

/// One pricing plan : name + per-meter unit prices + flat base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingPlan {
    /// Canonical plan identifier (`free`, `starter`, `pro`, `enterprise`).
    pub name: String,
    /// Display name shown in the portal.
    pub display_name: String,
    /// Monthly base fee in cents (charged regardless of usage).
    pub monthly_base_cents: PriceCents,
    /// Per-meter unit price map.
    pub unit_prices: HashMap<Meter, UnitPrice>,
}

impl PricingPlan {
    /// Look up the unit price for a meter under this plan. Returns
    /// `MeterNotPriced` if the meter is missing.
    pub fn price_for(&self, meter: Meter) -> Result<UnitPrice> {
        self.unit_prices
            .get(&meter)
            .copied()
            .ok_or_else(|| BillingError::MeterNotPriced {
                meter: meter.name().to_string(),
                plan: self.name.clone(),
            })
    }

    /// Compute the total cost for a usage map. Includes the
    /// `monthly_base_cents` regardless of usage. Skips meters absent
    /// from `usage`.
    pub fn total_cost(&self, usage: &HashMap<Meter, u64>) -> Result<PriceCents> {
        let mut total = self.monthly_base_cents;
        for (m, units) in usage {
            let price = self.price_for(*m)?;
            total = total.saturating_add(price.cost(*units));
        }
        Ok(total)
    }
}

/// Catalogue of pricing plans, keyed by canonical name.
#[derive(Debug, Clone, Default)]
pub struct PricingPlanCatalog {
    plans: HashMap<String, PricingPlan>,
}

impl PricingPlanCatalog {
    /// Construct from a list of plans.
    pub fn from_vec(plans: Vec<PricingPlan>) -> Self {
        let mut m = HashMap::new();
        for p in plans {
            m.insert(p.name.clone(), p);
        }
        Self { plans: m }
    }

    /// Look up a plan by canonical name.
    pub fn get(&self, name: &str) -> Result<&PricingPlan> {
        self.plans
            .get(name)
            .ok_or_else(|| BillingError::UnknownPlan(name.to_string()))
    }

    /// Returns true iff the catalogue contains a plan with that name.
    pub fn contains(&self, name: &str) -> bool {
        self.plans.contains_key(name)
    }

    /// Number of plans loaded.
    pub fn len(&self) -> usize {
        self.plans.len()
    }

    /// Returns `true` if no plan is loaded.
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    /// Iterate over plans in name order.
    pub fn iter(&self) -> impl Iterator<Item = &PricingPlan> + '_ {
        let mut sorted: Vec<_> = self.plans.values().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        sorted.into_iter()
    }
}

/// Construct the default M31 catalogue (free + starter + pro). The
/// `enterprise` plan is per-customer and populated from PG. Useful
/// for tests + bootstrap of staging clusters.
pub fn default_catalog() -> PricingPlanCatalog {
    fn map(pairs: &[(Meter, UnitPrice)]) -> HashMap<Meter, UnitPrice> {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(*k, *v);
        }
        // Fill missing meters with ZERO so plan validity is bounded.
        for meter in Meter::all() {
            m.entry(*meter).or_insert(UnitPrice::ZERO);
        }
        m
    }

    let free = PricingPlan {
        name: "free".into(),
        display_name: "Free".into(),
        monthly_base_cents: 0,
        unit_prices: map(&[(
            Meter::ClassifyRequests,
            UnitPrice {
                free_units: 10_000,
                price_cents_per_unit: 0,
            },
        )]),
    };
    let starter = PricingPlan {
        name: "starter".into(),
        display_name: "Starter".into(),
        monthly_base_cents: 4900,
        unit_prices: map(&[
            (
                Meter::ClassifyRequests,
                UnitPrice {
                    free_units: 50_000,
                    price_cents_per_unit: 1, // $0.001 / req
                },
            ),
            (
                Meter::AgentRunSeconds,
                UnitPrice {
                    free_units: 0,
                    price_cents_per_unit: 1,
                },
            ),
            (
                Meter::MultimodalVisionRequests,
                UnitPrice {
                    free_units: 0,
                    price_cents_per_unit: 5,
                },
            ),
        ]),
    };
    let pro = PricingPlan {
        name: "pro".into(),
        display_name: "Pro".into(),
        monthly_base_cents: 49900,
        unit_prices: map(&[
            (
                Meter::ClassifyRequests,
                UnitPrice {
                    free_units: 500_000,
                    price_cents_per_unit: 1, // matches starter, base fee carries the discount
                },
            ),
            (
                Meter::AgentRunSeconds,
                UnitPrice {
                    free_units: 0,
                    price_cents_per_unit: 1,
                },
            ),
            (
                Meter::MultimodalVisionRequests,
                UnitPrice {
                    free_units: 0,
                    price_cents_per_unit: 3,
                },
            ),
        ]),
    };
    PricingPlanCatalog::from_vec(vec![free, starter, pro])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_price_zero_is_free() {
        assert_eq!(UnitPrice::ZERO.cost(1_000_000), 0);
    }

    #[test]
    fn unit_price_cost_after_free_allowance() {
        let p = UnitPrice {
            free_units: 100,
            price_cents_per_unit: 2,
        };
        assert_eq!(p.cost(0), 0);
        assert_eq!(p.cost(100), 0);
        assert_eq!(p.cost(101), 2);
        assert_eq!(p.cost(200), 200);
    }

    #[test]
    fn plan_total_cost_includes_base_fee() {
        let plan = default_catalog().get("starter").unwrap().clone();
        let usage = HashMap::new();
        assert_eq!(plan.total_cost(&usage).unwrap(), 4900);
    }

    #[test]
    fn plan_total_cost_aggregates_meters() {
        let plan = default_catalog().get("starter").unwrap().clone();
        let mut usage = HashMap::new();
        usage.insert(Meter::ClassifyRequests, 100_000); // 50k free + 50k * 1 = $500
        usage.insert(Meter::MultimodalVisionRequests, 1_000); // 1k * 5 = $50
        let total = plan.total_cost(&usage).unwrap();
        assert_eq!(total, 4900 + 50_000 + 5_000);
    }

    #[test]
    fn catalog_returns_unknown_plan() {
        let cat = default_catalog();
        let err = cat.get("missing").unwrap_err();
        assert_eq!(err.error_code(), "billing.unknown_plan");
    }

    #[test]
    fn catalog_iter_is_alphabetic() {
        let cat = default_catalog();
        let names: Vec<_> = cat.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["free", "pro", "starter"]);
    }

    #[test]
    fn meter_not_priced_returns_typed_error() {
        let mut plan = default_catalog().get("starter").unwrap().clone();
        plan.unit_prices.clear(); // drop everything
        let mut usage = HashMap::new();
        usage.insert(Meter::ClassifyRequests, 1);
        let err = plan.total_cost(&usage).unwrap_err();
        assert!(matches!(err, BillingError::MeterNotPriced { .. }));
    }
}
