//! Deterministic A/B test framework for prompt templates.
//!
//! Assignments are deterministic per `(template_base_id, tenant_id)` —
//! a tenant in the v2 cohort always sees v2, never bouncing between
//! versions. This is critical for cache key stability (cf. M21
//! architecture § 9.1).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A single A/B cohort definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbCohort {
    /// Candidate template id (e.g. `mode2_anchor_gen_v2`).
    pub variant: String,
    /// Fraction of tenants in this cohort, [0, 1].
    pub weight: f64,
}

/// A table mapping `base_id` → ordered list of cohorts.
///
/// Default behaviour (empty table) : passthrough — `resolve("foo_v1", t)`
/// returns `"foo_v1"` regardless of tenant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AbAssignmentTable {
    /// Map base_id → cohorts.
    #[serde(default)]
    pub assignments: HashMap<String, Vec<AbCohort>>,
}

impl AbAssignmentTable {
    /// Returns the resolved template id for `(base_id, tenant_id)`.
    ///
    /// Algorithm :
    ///   1. Compute `score = sha256(base_id || "\0" || tenant_id) → u64`
    ///      → fraction in [0, 1).
    ///   2. Walk cohorts cumulatively ; return first whose cumulative
    ///      weight covers the score.
    ///   3. If no cohort covers (e.g. weights sum < 1), return `base_id`.
    pub fn resolve(&self, base_id: &str, tenant_id: &str) -> String {
        let cohorts = match self.assignments.get(base_id) {
            Some(c) if !c.is_empty() => c,
            _ => return base_id.to_string(),
        };

        let score = score_for(base_id, tenant_id);
        let mut cumulative = 0.0;
        for cohort in cohorts {
            cumulative += cohort.weight.max(0.0).min(1.0);
            if score < cumulative {
                return cohort.variant.clone();
            }
        }
        base_id.to_string()
    }

    /// Add a cohort (fluent builder).
    pub fn add_cohort(mut self, base_id: &str, variant: &str, weight: f64) -> Self {
        self.assignments
            .entry(base_id.to_string())
            .or_default()
            .push(AbCohort {
                variant: variant.to_string(),
                weight,
            });
        self
    }
}

fn score_for(base_id: &str, tenant_id: &str) -> f64 {
    let mut hasher = Sha256::new();
    hasher.update(base_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(tenant_id.as_bytes());
    let digest = hasher.finalize();
    // Take first 8 bytes as u64, normalise to [0, 1).
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let n = u64::from_be_bytes(bytes);
    (n as f64) / (u64::MAX as f64 + 1.0)
}
