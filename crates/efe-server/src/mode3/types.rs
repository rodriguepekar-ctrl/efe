//! Mode-3 types — request, response, config, convergence.

use serde::{Deserialize, Serialize};

/// Stakes tier for compliance gating (per-catalog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StakesTier {
    Low,
    Medium,
    High,
}

impl StakesTier {
    /// Parse from Postgres column. Default `Low` on unknown for
    /// backward-compat safety (low-stakes is the most permissive).
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "high" => StakesTier::High,
            "medium" => StakesTier::Medium,
            _ => StakesTier::Low,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StakesTier::Low => "low",
            StakesTier::Medium => "medium",
            StakesTier::High => "high",
        }
    }

    /// `true` if the tier requires human review per GDPR Art. 22(3) +
    /// AI Act Art. 14 default policy.
    pub fn requires_review_when_unconverged(&self) -> bool {
        matches!(self, StakesTier::High)
    }
}

/// Per-tenant Mode-3 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantMode3Config {
    pub tenant_id: String,
    pub enabled: bool,
    pub human_review_required: bool,
    pub default_max_turns: u32,
    pub max_max_turns: u32,
    pub default_threshold: f32,
    pub review_ttl_days: u32,
}

impl TenantMode3Config {
    /// Sane defaults for un-configured tenants — strict opt-in.
    pub fn default_for(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            enabled: false,
            human_review_required: false,
            default_max_turns: 3,
            max_max_turns: 5,
            default_threshold: 0.85,
            review_ttl_days: 7,
        }
    }
}

/// Mode-3 classify request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyMode3Request {
    pub tenant_id: String,
    pub catalog: String,
    pub bundle_version: String,
    pub sentence: String,
    pub sentence_redacted: String,
    /// Per-request override (must be ≤ tenant.max_max_turns).
    pub max_turns: Option<u32>,
    /// Per-request override (default tenant.default_threshold).
    pub threshold: Option<f32>,
    /// Top-K initial candidates (default 10).
    pub top_k_anchors: Option<u32>,
}

/// Mode-3 classify response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyMode3Response {
    /// `None` only if max_turns hit AND queued for human review.
    pub final_label: Option<String>,
    pub final_confidence: Option<f32>,
    pub mode: String, // always "MODE_3"
    pub total_turns: u32,
    pub total_cost_usd: f64,
    pub converged: bool,
    pub requires_human_review: bool,
    /// UUID v4 if review queued, else None.
    pub review_token: Option<String>,
    /// Set if Mode-3 fell back (e.g. all providers failed turn 1).
    pub degraded_reason: Option<String>,
    /// Audit chain row id for `efe_get_concept_stack(audit_id)`.
    pub audit_id: Option<i64>,
}

/// Convergence check result after each turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvergenceResult {
    /// `swarm_confidence >= threshold` reached. STOP, return final.
    Converged,
    /// `turn == max_turns`. STOP, may queue for human review.
    MaxTurnsReached,
    /// Below threshold, more turns available. CONTINUE.
    Continue,
}

impl ConvergenceResult {
    pub fn check(swarm_confidence: Option<f32>, threshold: f32, turn: u32, max_turns: u32) -> Self {
        if let Some(conf) = swarm_confidence {
            if conf >= threshold {
                return ConvergenceResult::Converged;
            }
        }
        if turn >= max_turns {
            return ConvergenceResult::MaxTurnsReached;
        }
        ConvergenceResult::Continue
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, ConvergenceResult::Continue)
    }
}
