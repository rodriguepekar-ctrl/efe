//! Prometheus metrics for the active learning crate.
//!
//! Names are stable (consumed by Grafana dashboards + alert rules) —
//! never rename without bumping the dashboard version.

use once_cell::sync::Lazy;
use prometheus::{
    register_int_counter, register_int_counter_vec,
    IntCounter, IntCounterVec,
};

// ── Sampler ─────────────────────────────────────────────────────────

pub static AL_SAMPLES_ENQUEUED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_samples_enqueued_total",
        "Number of samples enqueued for labeling",
        &["tenant", "catalog"],
    )
    .expect("metric register : efe_al_samples_enqueued_total")
});

pub static AL_QUEUE_BACK_PRESSURE_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_queue_back_pressure_total",
        "Times the sampler skipped enqueueing due to back-pressure",
        &["tenant"],
    )
    .expect("metric register : efe_al_queue_back_pressure_total")
});

// ── LLM worker ──────────────────────────────────────────────────────

pub static AL_LABELS_PRODUCED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_labels_produced_total",
        "LLM labelings produced, by tenant + outcome (success | fail)",
        &["tenant", "outcome"],
    )
    .expect("metric register : efe_al_labels_produced_total")
});

pub static AL_LLM_WORKER_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_llm_worker_errors_total",
        "LLM worker errors, by tenant",
        &["tenant"],
    )
    .expect("metric register : efe_al_llm_worker_errors_total")
});

pub static AL_LLM_WORKER_BUDGET_BLOCKED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_llm_worker_budget_blocked_total",
        "Times the LLM worker skipped a tenant due to budget exhaustion",
        &["tenant"],
    )
    .expect("metric register : efe_al_llm_worker_budget_blocked_total")
});

// ── Review ──────────────────────────────────────────────────────────

pub static AL_REVIEWS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_reviews_total",
        "Human review decisions on gold label proposals",
        &["tenant", "decision"],
    )
    .expect("metric register : efe_al_reviews_total")
});

pub static AL_AUTO_CONFIRM_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_auto_confirm_total",
        "Auto-confirm decisions (gate result : allowed | blocked)",
        &["tenant", "outcome"],
    )
    .expect("metric register : efe_al_auto_confirm_total")
});

// ── Retune ──────────────────────────────────────────────────────────

pub static AL_RETUNE_RUNS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_retune_runs_total",
        "Bundle retune runs, by tenant + status",
        &["tenant", "status"],
    )
    .expect("metric register : efe_al_retune_runs_total")
});

pub static AL_CENTROID_DRIFT_ABORTED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_centroid_drift_aborted_total",
        "Retune runs aborted due to centroid drift > threshold",
        &["tenant", "catalog"],
    )
    .expect("metric register : efe_al_centroid_drift_aborted_total")
});

// ── Promotion ──────────────────────────────────────────────────────

pub static AL_PROMOTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_promotions_total",
        "Bundle promotions, by tenant + path (auto | operator | gdpr_force)",
        &["tenant", "path"],
    )
    .expect("metric register : efe_al_promotions_total")
});

pub static AL_PROMOTIONS_BLOCKED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_promotions_blocked_total",
        "Auto-promotion blocked, by tenant + reason",
        &["tenant", "reason"],
    )
    .expect("metric register : efe_al_promotions_blocked_total")
});

// ── GDPR ────────────────────────────────────────────────────────────

pub static AL_GDPR_ART17_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_al_gdpr_art17_requests_total",
        "GDPR Art. 17 erasure requests received, by tenant + urgency",
        &["tenant", "urgency"],
    )
    .expect("metric register : efe_al_gdpr_art17_requests_total")
});

pub static AL_GDPR_ART17_COMPLETED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_al_gdpr_art17_completed_total",
        "GDPR Art. 17 erasure requests completed (cascade rebuild done)",
    )
    .expect("metric register : efe_al_gdpr_art17_completed_total")
});

pub static AL_GDPR_ART17_SLA_BREACH_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_al_gdpr_art17_sla_breach_total",
        "GDPR Art. 17 requests that breached SLA — must be 0",
    )
    .expect("metric register : efe_al_gdpr_art17_sla_breach_total")
});
