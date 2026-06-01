// SPDX-License-Identifier: UNLICENSED
//! # `efe-billing` — M31 Stripe billing plane.
//!
//! Five sub-modules :
//!
//! - [`error`]            — typed error taxonomy.
//! - [`stripe_client`]    — thin async wrapper over `api.stripe.com`.
//! - [`pricing`]          — pricing-plan cache + cost computation.
//! - [`invoice_builder`]  — month-end invoice generation.
//! - [`reconciliation`]   — drift detection between local mirror +
//!   Stripe.
//! - [`budget`]           — soft / hard cap evaluation.
//! - [`metrics`]          — Prometheus metrics.
//!
//! ## Architectural invariants
//!
//! 1. **No PAN ever stored.** Stripe is the system of record for card
//!    data ; we keep at most `last4` + `brand`.
//! 2. **Idempotent retries.** Every Stripe API call uses an
//!    idempotency key derived from `(tenant, period, action)` so a
//!    network blip cannot create duplicate invoices.
//! 3. **Webhook signature verified.** Every inbound Stripe webhook
//!    is HMAC-verified before any side effect.
//! 4. **Audit trail.** Plan changes, invoice finalizations, and
//!    budget dismissals all write a HMAC audit row.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod budget;
pub mod error;
pub mod invoice_builder;
pub mod metrics;
pub mod pricing;
pub mod reconciliation;
pub mod stripe_client;
pub mod webhook;

pub use budget::{
    evaluate_budget, BudgetAlert, BudgetEvaluation, BudgetSeverity, NotificationChannel,
};
pub use error::{BillingError, Result};
pub use invoice_builder::{build_line_items, InvoiceLineItem, MonthlyInvoiceDraft};
pub use pricing::{PriceCents, PricingPlan, PricingPlanCatalog, UnitPrice};
pub use reconciliation::{ReconciliationOutcome, ReconciliationReport};
pub use stripe_client::{
    StripeClient, StripeClientConfig, StripeCustomer, StripeInvoice, StripeInvoiceStatus,
    StripeSubscription,
};
pub use webhook::{verify_stripe_signature, StripeEvent, WebhookConfig};

/// Static crate identifier.
pub const CRATE_NAME_VERSION: &str = concat!("efe-billing/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_well_formed() {
        assert!(CRATE_NAME_VERSION.starts_with("efe-billing/"));
    }
}
