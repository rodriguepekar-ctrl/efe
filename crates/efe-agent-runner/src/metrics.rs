//! Prometheus metrics for the agent runtime crate.
//!
//! The `sandbox_escape_attempts_total` counter is the most critical
//! metric in M25 : any non-zero value triggers PagerDuty P0 immediately.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_counter_vec,
    register_int_gauge_vec, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec,
};

// ── Run lifecycle ───────────────────────────────────────────────────

pub static AGENT_RUNS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_runs_total",
        "Agent runs by tenant + final status",
        &["tenant", "status"],
    )
    .expect("register efe_agent_runs_total")
});

pub static AGENT_RUNS_ACTIVE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_agent_runs_active",
        "Currently active agent runs by tenant",
        &["tenant"],
    )
    .expect("register efe_agent_runs_active")
});

pub static AGENT_RUN_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_agent_run_duration_seconds",
        "Agent run wall-clock duration by tenant + outcome",
        &["tenant", "outcome"],
        // Buckets : up to ~4h
        vec![10.0, 60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0, 14400.0],
    )
    .expect("register efe_agent_run_duration_seconds")
});

// ── Steps ───────────────────────────────────────────────────────────

pub static AGENT_STEP_COUNT: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_step_count_total",
        "Steps emitted by agent runs, by tenant + kind (llm_call / tool_call / approval / final)",
        &["tenant", "kind"],
    )
    .expect("register efe_agent_step_count_total")
});

// ── Cost ────────────────────────────────────────────────────────────

pub static AGENT_COST_CENTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_cost_cents_total",
        "Cumulative agent runtime cost in cents, by tenant",
        &["tenant"],
    )
    .expect("register efe_agent_cost_cents_total")
});

pub static AGENT_COST_CAP_HIT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_cost_cap_hit_total",
        "Number of agent runs SIGTERMed for cost cap, by tenant",
        &["tenant"],
    )
    .expect("register efe_agent_cost_cap_hit_total")
});

// ── Approval ────────────────────────────────────────────────────────

pub static AGENT_APPROVAL_RESPONSE_LATENCY: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_agent_approval_response_latency_seconds",
        "Time from approval request to response",
        &["tenant", "decision"],
        vec![1.0, 60.0, 300.0, 900.0, 3600.0, 14400.0, 86400.0],
    )
    .expect("register efe_agent_approval_response_latency_seconds")
});

pub static AGENT_APPROVAL_EXPIRED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_approval_expired_total",
        "Approval requests that expired without response",
        &["tenant"],
    )
    .expect("register efe_agent_approval_expired_total")
});

// ── Sandbox security ────────────────────────────────────────────────

/// **CRITICAL** : ANY non-zero value triggers PagerDuty P0.
pub static AGENT_SANDBOX_ESCAPE_ATTEMPTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_sandbox_escape_attempts_total",
        "SECURITY-CRITICAL : sandbox escape attempts detected. Must always be 0 in production.",
        &["tenant", "kind"],  // kind : ptrace, syscall_denied, network_egress_denied, image_swap, ...
    )
    .expect("register efe_agent_sandbox_escape_attempts_total")
});

pub static AGENT_TOOL_REJECTED_WHITELIST_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_tool_rejected_whitelist_total",
        "Tool calls rejected for not being in the run's whitelist (LLM hallucination or attack)",
        &["tenant", "tool"],
    )
    .expect("register efe_agent_tool_rejected_whitelist_total")
});

pub static AGENT_IMAGE_REJECTED_WHITELIST_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_agent_image_rejected_whitelist_total",
        "Pod admissions rejected because image is not in whitelist",
    )
    .expect("register efe_agent_image_rejected_whitelist_total")
});

// ── microVM pool ───────────────────────────────────────────────────

pub static AGENT_MICROVM_POOL_SIZE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_agent_microvm_pool_size",
        "microVM pool size per tenant",
        &["tenant", "state"],  // state : idle, running, draining
    )
    .expect("register efe_agent_microvm_pool_size")
});

pub static AGENT_MICROVM_PROVISION_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_agent_microvm_provision_duration_seconds",
        "microVM provisioning (Firecracker spawn) duration",
        &["tenant", "outcome"],
        vec![0.05, 0.1, 0.2, 0.5, 1.0, 5.0, 30.0],
    )
    .expect("register efe_agent_microvm_provision_duration_seconds")
});

// ── Failure modes ───────────────────────────────────────────────────

pub static AGENT_FAILURE_RATE_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_failure_rate_total",
        "Agent run failures by tenant + reason",
        &["tenant", "reason"],
    )
    .expect("register efe_agent_failure_rate_total")
});

// ── M26 metrics ─────────────────────────────────────────────────────

/// Histogram of Firecracker REST API call durations.
pub static AGENT_FIRECRACKER_API_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_agent_firecracker_api_duration_seconds",
        "Firecracker UDS REST API call duration by endpoint + status",
        &["api", "status"],
        vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 30.0],
    )
    .expect("register efe_agent_firecracker_api_duration_seconds")
});

/// Counter incremented when the `MicrovmLease` Drop impl recovers a
/// slot that would otherwise have leaked (uncommitted lease).
pub static AGENT_MICROVM_LEAK_RECOVERED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_agent_microvm_leak_recovered_total",
        "M26 RAII guard salvaged a non-committed MicrovmLease",
    )
    .expect("register efe_agent_microvm_leak_recovered_total")
});

/// Counter for the periodic reaper sweeps.
pub static AGENT_MICROVM_REAPED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_agent_microvm_reaped_total",
        "Reaper goroutine killed an orphan microVM slot, by tenant",
        &["tenant"],
    )
    .expect("register efe_agent_microvm_reaped_total")
});

/// HMAC verification failures on the audit chain. Must always be 0.
pub static AGENT_AUDIT_HMAC_VERIFY_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_agent_audit_hmac_verify_failures_total",
        "SECURITY-CRITICAL : audit chain HMAC verification failures. Must always be 0 in production.",
    )
    .expect("register efe_agent_audit_hmac_verify_failures_total")
});

/// Histogram of cost-cap-exceeded → SIGTERM duration.
pub static AGENT_COST_CAP_TERMINATE_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_agent_cost_cap_terminate_duration_seconds",
        "Time from cost cap exceeded to SIGTERM sent to microVM",
        &["tenant"],
        vec![0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0],
    )
    .expect("register efe_agent_cost_cap_terminate_duration_seconds")
});

/// Gauge : oldest pending GDPR Art. 17 cascade per tenant (seconds).
pub static AGENT_GDPR_CASCADE_OLDEST_PENDING_SECONDS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_agent_gdpr_cascade_oldest_pending_seconds",
        "Age in seconds of the oldest pending GDPR Art. 17 cascade per tenant",
        &["tenant"],
    )
    .expect("register efe_agent_gdpr_cascade_oldest_pending_seconds")
});
