//! Prometheus metrics for the curation crate.
//!
//! All counters / histograms are registered into the global default
//! registry via `prometheus::register_*` macros. Names are stable
//! (consumed by Grafana dashboards + alert rules) — never rename
//! without bumping the dashboard version.

use once_cell::sync::Lazy;
use prometheus::{
    register_int_counter, register_int_counter_vec, IntCounter, IntCounterVec,
};

// ── Tool calls + idempotency ─────────────────────────────────────────

pub static CURATION_TOOL_CALLS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_tool_calls_total",
        "Calls to curation tools, labelled by tenant + tool + outcome",
        &["tenant", "tool", "outcome"],
    )
    .expect("metric register : efe_curation_tool_calls_total")
});

pub static CURATION_TOOL_IDEMPOTENCY_HITS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_idempotency_hits_total",
        "Idempotency key hits avoiding redundant LLM calls",
        &["tenant", "tool"],
    )
    .expect("metric register : efe_curation_idempotency_hits_total")
});

// ── Reviews + promotions ─────────────────────────────────────────────

pub static CURATION_REVIEWS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_reviews_total",
        "Curation review decisions",
        &["tenant", "decision"],
    )
    .expect("metric register : efe_curation_reviews_total")
});

pub static CURATION_PROMOTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_promotions_total",
        "Taxonomy version promotions",
        &["tenant", "outcome"],
    )
    .expect("metric register : efe_curation_promotions_total")
});

// ── Drift detection ─────────────────────────────────────────────────

pub static DRIFT_EVENTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_drift_events_total",
        "Number of drift events detected, labelled by tenant + severity",
        &["tenant", "severity"],
    )
    .expect("metric register : efe_curation_drift_events_total")
});

pub static DRIFT_NO_EVENT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_curation_drift_no_event_total",
        "Number of drift checks with no event triggered",
        &["tenant"],
    )
    .expect("metric register : efe_curation_drift_no_event_total")
});

pub static DRIFT_DETECTION_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_curation_drift_errors_total",
        "Number of drift detection errors",
    )
    .expect("metric register : efe_curation_drift_errors_total")
});

// ── Cost / budget ───────────────────────────────────────────────────

pub static CURATION_BUDGET_EXCEEDED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_curation_budget_exceeded_total",
        "Budget exceeded events",
    )
    .expect("metric register : efe_curation_budget_exceeded_total")
});
