// SPDX-License-Identifier: UNLICENSED
//! M31 — Monthly invoice draft builder.
//!
//! Walks the `usage_rollup_day` rows for a billing period, applies
//! the active pricing plan, and produces a `MonthlyInvoiceDraft`
//! ready to be pushed into Stripe via `StripeClient::create_invoice_item`
//! + `create_invoice` + `finalize_invoice`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use efe_metering::{Meter, RollupRow};

use crate::error::{BillingError, Result};
use crate::pricing::{PriceCents, PricingPlan};

/// One line item in the draft invoice. Maps 1:1 to a Stripe
/// `InvoiceItem`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLineItem {
    /// Meter name (for the description).
    pub meter: Meter,
    /// Number of units consumed during the period.
    pub units: u64,
    /// Computed cost in cents (already deducts free allowance).
    pub amount_cents: PriceCents,
    /// Human-readable description.
    pub description: String,
}

/// Draft invoice for one tenant + one billing period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyInvoiceDraft {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Plan identifier in effect during the period.
    pub plan_name: String,
    /// Period start (inclusive, ISO-8601 date YYYY-MM-DD).
    pub period_start: String,
    /// Period end (inclusive, ISO-8601 date YYYY-MM-DD).
    pub period_end: String,
    /// Monthly base fee in cents.
    pub base_fee_cents: PriceCents,
    /// Per-meter line items.
    pub line_items: Vec<InvoiceLineItem>,
    /// Sum of base fee + line items.
    pub total_cents: PriceCents,
}

/// Aggregate a stream of day-rollup rows into a single
/// `usage_by_meter` map. **Public for direct unit-testing.**
#[must_use]
pub fn aggregate_rollups(rows: &[RollupRow]) -> HashMap<Meter, u64> {
    let mut m: HashMap<Meter, u64> = HashMap::new();
    for r in rows {
        let entry = m.entry(r.meter).or_insert(0);
        *entry = entry.saturating_add(r.value);
    }
    m
}

/// Build line items from rollups + a pricing plan. Returns
/// (line_items, total_billable_amount).
///
/// **Pure function** — no I/O, no Stripe call. The downstream
/// invoice issuer pushes the line items via the Stripe client.
pub fn build_line_items(
    rollups: &[RollupRow],
    plan: &PricingPlan,
) -> Result<(Vec<InvoiceLineItem>, PriceCents)> {
    let usage = aggregate_rollups(rollups);
    let mut items = Vec::new();
    let mut billable = 0_u64;
    // Iterate over Meter::all() so the output order is stable.
    for meter in Meter::all() {
        let units = *usage.get(meter).unwrap_or(&0);
        if units == 0 {
            continue;
        }
        let price = plan.price_for(*meter)?;
        let amount = price.cost(units);
        if amount == 0 {
            // Free-allowance line items are not pushed to Stripe — they
            // would create $0 invoice items which are valid but noisy.
            continue;
        }
        items.push(InvoiceLineItem {
            meter: *meter,
            units,
            amount_cents: amount,
            description: format!(
                "{} : {} units × ${:.4} (after {} free)",
                meter.name(),
                units,
                (price.price_cents_per_unit as f64) / 100.0,
                price.free_units
            ),
        });
        billable = billable.saturating_add(amount);
    }
    Ok((items, billable))
}

/// Top-level helper : build a full `MonthlyInvoiceDraft` from
/// rollups + plan + period metadata.
pub fn build_draft(
    tenant_id: &str,
    plan: &PricingPlan,
    period_start: &str,
    period_end: &str,
    rollups: &[RollupRow],
) -> Result<MonthlyInvoiceDraft> {
    if period_start > period_end {
        return Err(BillingError::Internal {
            reason: format!("period_start {period_start} > period_end {period_end}"),
        });
    }
    let (line_items, billable) = build_line_items(rollups, plan)?;
    let total = plan.monthly_base_cents.saturating_add(billable);
    Ok(MonthlyInvoiceDraft {
        tenant_id: tenant_id.to_string(),
        plan_name: plan.name.clone(),
        period_start: period_start.to_string(),
        period_end: period_end.to_string(),
        base_fee_cents: plan.monthly_base_cents,
        line_items,
        total_cents: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::default_catalog;

    fn row(meter: Meter, value: u64) -> RollupRow {
        RollupRow {
            tenant_id: "acme".into(),
            period_start: "2026-05-11".into(),
            meter,
            value,
        }
    }

    #[test]
    fn aggregate_rollups_sums_by_meter() {
        let rows = vec![
            row(Meter::ClassifyRequests, 10),
            row(Meter::ClassifyRequests, 5),
            row(Meter::WebhookDeliveries, 2),
        ];
        let m = aggregate_rollups(&rows);
        assert_eq!(m[&Meter::ClassifyRequests], 15);
        assert_eq!(m[&Meter::WebhookDeliveries], 2);
    }

    #[test]
    fn build_line_items_skips_zero_amount_meters() {
        let plan = default_catalog().get("starter").unwrap().clone();
        // 1k requests — all within the 50k free allowance ⇒ no line.
        let rows = vec![row(Meter::ClassifyRequests, 1_000)];
        let (items, billable) = build_line_items(&rows, &plan).unwrap();
        assert!(items.is_empty());
        assert_eq!(billable, 0);
    }

    #[test]
    fn build_line_items_includes_billable_meters() {
        let plan = default_catalog().get("starter").unwrap().clone();
        // 100k requests : 50k free + 50k × 1 cent = $500.
        let rows = vec![row(Meter::ClassifyRequests, 100_000)];
        let (items, billable) = build_line_items(&rows, &plan).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].meter, Meter::ClassifyRequests);
        assert_eq!(items[0].units, 100_000);
        assert_eq!(items[0].amount_cents, 50_000);
        assert_eq!(billable, 50_000);
    }

    #[test]
    fn build_draft_period_validation() {
        let plan = default_catalog().get("starter").unwrap().clone();
        let err = build_draft("acme", &plan, "2026-05-31", "2026-05-01", &[]).unwrap_err();
        assert!(matches!(err, BillingError::Internal { .. }));
    }

    #[test]
    fn build_draft_includes_base_fee_when_zero_usage() {
        let plan = default_catalog().get("starter").unwrap().clone();
        let draft = build_draft("acme", &plan, "2026-05-01", "2026-05-31", &[]).unwrap();
        assert_eq!(draft.total_cents, 4900);
        assert!(draft.line_items.is_empty());
    }

    #[test]
    fn build_draft_sums_base_plus_lines() {
        let plan = default_catalog().get("starter").unwrap().clone();
        let rows = vec![row(Meter::ClassifyRequests, 100_000)];
        let draft = build_draft("acme", &plan, "2026-05-01", "2026-05-31", &rows).unwrap();
        assert_eq!(draft.total_cents, 4900 + 50_000);
    }
}
