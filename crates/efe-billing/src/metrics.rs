// SPDX-License-Identifier: UNLICENSED
//! M31 — Billing-plane Prometheus metrics.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec,
};

/// Stripe API duration in milliseconds. Labels : `endpoint`, `outcome`.
pub static STRIPE_API_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_stripe_api_duration_ms",
        "Stripe API round-trip duration in milliseconds.",
        &["endpoint", "outcome"],
        vec![5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 5_000.0, 10_000.0]
    )
    .expect("register efe_stripe_api_duration_ms")
});

/// Stripe error count. Labels : `endpoint`, `kind`.
pub static STRIPE_API_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_stripe_api_errors_total",
        "Stripe API error count by endpoint + kind.",
        &["endpoint", "kind"]
    )
    .expect("register efe_stripe_api_errors_total")
});

/// Inbound webhook intake counter. Labels : `event_type`, `outcome`.
pub static STRIPE_WEBHOOK_RECEIVED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_stripe_webhook_received_total",
        "Stripe webhook events received.",
        &["event_type", "outcome"]
    )
    .expect("register efe_stripe_webhook_received_total")
});

/// Invoice lifecycle counter. Labels : `status`.
pub static INVOICE_STATUS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_invoice_status_total",
        "Invoice lifecycle transitions.",
        &["status"]
    )
    .expect("register efe_invoice_status_total")
});

/// Budget exceeded counter. Labels : `severity`.
pub static BUDGET_EXCEEDED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_budget_exceeded_total",
        "Budget alerts fired.",
        &["severity"]
    )
    .expect("register efe_budget_exceeded_total")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_all_metrics() {
        STRIPE_API_DURATION_MS
            .with_label_values(&["create_customer", "ok"])
            .observe(50.0);
        STRIPE_API_ERRORS_TOTAL
            .with_label_values(&["create_invoice", "timeout"])
            .inc();
        STRIPE_WEBHOOK_RECEIVED_TOTAL
            .with_label_values(&["invoice.paid", "ok"])
            .inc();
        INVOICE_STATUS_TOTAL
            .with_label_values(&["finalized"])
            .inc();
        BUDGET_EXCEEDED_TOTAL
            .with_label_values(&["soft"])
            .inc();
    }
}
