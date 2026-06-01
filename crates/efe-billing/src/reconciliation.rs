// SPDX-License-Identifier: UNLICENSED
//! M31 — Local-mirror ↔ Stripe drift detection.
//!
//! Runs every 6 h. For each (tenant, current month) pair, compares :
//!
//! 1. **Local mirror** : `billing_invoice` rows for this period.
//! 2. **Stripe**       : `Invoice.list?customer={id}&limit=10`.
//!
//! Reports any drift via `ReconciliationOutcome`. Drift is rare in
//! steady state but can happen when :
//!
//! - A webhook was lost (NATS down at the time).
//! - A Stripe operator manually voided an invoice.
//! - A mid-cycle plan change confused our state machine.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::stripe_client::StripeInvoice;

/// Reconciliation outcome for one tenant + period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReconciliationOutcome {
    /// Mirror matches Stripe exactly.
    Match,
    /// Mirror is missing an invoice that Stripe has.
    MissingLocal {
        /// Stripe `in_...` id.
        stripe_invoice_id: String,
    },
    /// Mirror has an invoice that Stripe no longer has.
    MissingRemote {
        /// Local mirror invoice id.
        local_invoice_id: String,
    },
    /// Both sides have the invoice but the totals differ.
    AmountDrift {
        /// Stripe `in_...` id.
        stripe_invoice_id: String,
        /// Local total in cents.
        local_total: i64,
        /// Remote total in cents.
        remote_total: i64,
    },
    /// Status divergence (e.g. local says `draft`, Stripe says `paid`).
    StatusDrift {
        /// Stripe `in_...` id.
        stripe_invoice_id: String,
        /// Local status.
        local_status: String,
        /// Remote status.
        remote_status: String,
    },
}

/// Report aggregating outcomes for one reconciliation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// Number of tenants checked.
    pub tenants_checked: usize,
    /// Non-empty outcomes per tenant.
    pub outcomes: Vec<(String, ReconciliationOutcome)>,
}

impl ReconciliationReport {
    /// `true` iff every checked tenant matched Stripe.
    pub fn is_clean(&self) -> bool {
        self.outcomes
            .iter()
            .all(|(_, o)| matches!(o, ReconciliationOutcome::Match))
    }
}

/// Mirror representation of a `billing_invoice` row (subset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalInvoice {
    /// Local UUID.
    pub id: String,
    /// Stripe `in_...` id (FK to the remote object).
    pub stripe_invoice_id: String,
    /// Status (local string mirror of Stripe status).
    pub status: String,
    /// Total in cents.
    pub total: i64,
}

/// Compare a local view to a Stripe-side list of invoices for a
/// single tenant. Returns the first drift detected (or
/// `Match` if all agree).
pub fn compare_tenant(
    local: &[LocalInvoice],
    remote: &[StripeInvoice],
) -> Result<ReconciliationOutcome> {
    // Index by Stripe id.
    let mut local_idx = std::collections::HashMap::new();
    for l in local {
        local_idx.insert(l.stripe_invoice_id.clone(), l);
    }
    let mut remote_idx = std::collections::HashMap::new();
    for r in remote {
        remote_idx.insert(r.id.clone(), r);
    }
    // 1) Remote-not-local.
    for r in remote {
        if !local_idx.contains_key(&r.id) {
            return Ok(ReconciliationOutcome::MissingLocal {
                stripe_invoice_id: r.id.clone(),
            });
        }
    }
    // 2) Local-not-remote.
    for l in local {
        if !remote_idx.contains_key(&l.stripe_invoice_id) {
            return Ok(ReconciliationOutcome::MissingRemote {
                local_invoice_id: l.id.clone(),
            });
        }
    }
    // 3) Per-invoice diff.
    for l in local {
        if let Some(r) = remote_idx.get(&l.stripe_invoice_id) {
            if l.total != r.total {
                return Ok(ReconciliationOutcome::AmountDrift {
                    stripe_invoice_id: r.id.clone(),
                    local_total: l.total,
                    remote_total: r.total,
                });
            }
            let remote_status = serde_json::to_value(r.status)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown".into());
            if l.status != remote_status {
                return Ok(ReconciliationOutcome::StatusDrift {
                    stripe_invoice_id: r.id.clone(),
                    local_status: l.status.clone(),
                    remote_status,
                });
            }
        }
    }
    Ok(ReconciliationOutcome::Match)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stripe_client::StripeInvoiceStatus;

    fn local(id: &str, stripe_id: &str, status: &str, total: i64) -> LocalInvoice {
        LocalInvoice {
            id: id.into(),
            stripe_invoice_id: stripe_id.into(),
            status: status.into(),
            total,
        }
    }

    fn remote(stripe_id: &str, status: StripeInvoiceStatus, total: i64) -> StripeInvoice {
        StripeInvoice {
            id: stripe_id.into(),
            customer: "cus_x".into(),
            status,
            total,
            currency: "usd".into(),
            hosted_invoice_url: None,
            period_start: 0,
            period_end: 0,
        }
    }

    #[test]
    fn match_when_identical() {
        let l = vec![local("uuid", "in_1", "paid", 1000)];
        let r = vec![remote("in_1", StripeInvoiceStatus::Paid, 1000)];
        let out = compare_tenant(&l, &r).unwrap();
        assert_eq!(out, ReconciliationOutcome::Match);
    }

    #[test]
    fn missing_local_detected() {
        let l: Vec<LocalInvoice> = vec![];
        let r = vec![remote("in_1", StripeInvoiceStatus::Paid, 1000)];
        let out = compare_tenant(&l, &r).unwrap();
        assert_eq!(
            out,
            ReconciliationOutcome::MissingLocal {
                stripe_invoice_id: "in_1".into()
            }
        );
    }

    #[test]
    fn missing_remote_detected() {
        let l = vec![local("uuid", "in_1", "paid", 1000)];
        let r: Vec<StripeInvoice> = vec![];
        let out = compare_tenant(&l, &r).unwrap();
        assert_eq!(
            out,
            ReconciliationOutcome::MissingRemote {
                local_invoice_id: "uuid".into()
            }
        );
    }

    #[test]
    fn amount_drift_detected() {
        let l = vec![local("uuid", "in_1", "paid", 1000)];
        let r = vec![remote("in_1", StripeInvoiceStatus::Paid, 2000)];
        let out = compare_tenant(&l, &r).unwrap();
        assert!(matches!(out, ReconciliationOutcome::AmountDrift { .. }));
    }

    #[test]
    fn status_drift_detected() {
        let l = vec![local("uuid", "in_1", "draft", 1000)];
        let r = vec![remote("in_1", StripeInvoiceStatus::Paid, 1000)];
        let out = compare_tenant(&l, &r).unwrap();
        assert!(matches!(out, ReconciliationOutcome::StatusDrift { .. }));
    }

    #[test]
    fn report_is_clean_flag() {
        let report = ReconciliationReport {
            tenants_checked: 2,
            outcomes: vec![
                ("acme".into(), ReconciliationOutcome::Match),
                ("foo".into(), ReconciliationOutcome::Match),
            ],
        };
        assert!(report.is_clean());

        let report = ReconciliationReport {
            tenants_checked: 1,
            outcomes: vec![(
                "acme".into(),
                ReconciliationOutcome::MissingLocal {
                    stripe_invoice_id: "in_1".into(),
                },
            )],
        };
        assert!(!report.is_clean());
    }
}
