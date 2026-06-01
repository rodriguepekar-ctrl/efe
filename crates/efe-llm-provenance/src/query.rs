// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : query types — `DateRange`, `CostSummary`,
//! `ModelCostRow`, `ModelHashMismatchEvent`.

use std::time::{Duration, SystemTime};

use efe_nano_sdk::{Blake3Hash, ModelIdentity, TenantId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  DateRange
// ─────────────────────────────────────────────────────────────────────

/// **N17** : closed-open `[from, to)` window over `SystemTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub from: SystemTime,
    pub to: SystemTime,
}

impl DateRange {
    /// **N17** : Construct a range covering the last `n` minutes.
    ///
    /// The upper bound is `now + 60 s` (NOT `now`) — a 60-second
    /// future buffer that tolerates clock skew between the caller's
    /// machine and the store's `recorded_at` timestamp.  This is
    /// the standard pattern for "since X minutes ago" semantics in
    /// the N16 + N17 query paths.
    pub fn since_minutes_ago(n: u64) -> Self {
        let now = SystemTime::now();
        Self {
            from: now - Duration::from_secs(n * 60),
            to: now + Duration::from_secs(60),
        }
    }

    /// **N17** : Construct a range covering the last `n` days.
    ///
    /// Same +60 s upper-bound buffer semantics as
    /// [`Self::since_minutes_ago`].
    pub fn since_days_ago(n: u64) -> Self {
        let now = SystemTime::now();
        Self {
            from: now - Duration::from_secs(n * 24 * 60 * 60),
            to: now + Duration::from_secs(60),
        }
    }

    /// **N17** : True if `t` falls within the range.
    pub fn contains(&self, t: SystemTime) -> bool {
        t >= self.from && t < self.to
    }
}

// ─────────────────────────────────────────────────────────────────────
//  CostSummary
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : per-model cost row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCostRow {
    pub model_identity: ModelIdentity,
    pub cost_cents: f64,
    pub call_count: u64,
    pub client_estimated_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reported_tokens: Option<u64>,
}

/// **Sprint 9 — N17** : aggregate cost summary returned by the
/// store's `cost_summary` endpoint.  The `by_model` field is a
/// `Vec<ModelCostRow>` rather than a HashMap because JSON object
/// keys must be strings — `ModelIdentity` is a struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSummary {
    pub by_model: Vec<ModelCostRow>,
    pub total_cents: f64,
    pub total_client_estimated_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_provider_reported_tokens: Option<u64>,
    /// `(provider - client) / client` ; None when no provider counts
    /// or zero client total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_ratio: Option<f64>,
    pub call_count: u64,
}

impl CostSummary {
    /// **N17** : Find the row for a specific model (linear scan ;
    /// `by_model` is typically small — bounded by tenant's active
    /// model count).
    pub fn row_for(&self, model: &ModelIdentity) -> Option<&ModelCostRow> {
        self.by_model
            .iter()
            .find(|r| r.model_identity == *model)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ModelHashMismatchEvent
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : recorded on every F3 detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelHashMismatchEvent {
    pub event_id: Uuid,
    pub tenant_id: TenantId,
    pub model_identity: ModelIdentity,
    pub expected_hash: Blake3Hash,
    pub actual_hash: Blake3Hash,
    pub detected_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_range_since_minutes_contains_now() {
        let r = DateRange::since_minutes_ago(10);
        assert!(r.contains(SystemTime::now()));
    }

    #[test]
    fn date_range_since_days_contains_now() {
        let r = DateRange::since_days_ago(7);
        assert!(r.contains(SystemTime::now()));
    }

    #[test]
    fn date_range_contains_boundary_semantics() {
        let now = SystemTime::now();
        let r = DateRange {
            from: now,
            to: now + Duration::from_secs(60),
        };
        assert!(r.contains(now));
        assert!(!r.contains(now + Duration::from_secs(60)));
    }

    #[test]
    fn cost_summary_serde_round_trip() {
        let row = ModelCostRow {
            model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024"),
            cost_cents: 100.0,
            call_count: 5,
            client_estimated_tokens: 1500,
            provider_reported_tokens: Some(1550),
        };
        let s = CostSummary {
            by_model: vec![row.clone()],
            total_cents: 100.0,
            total_client_estimated_tokens: 1500,
            total_provider_reported_tokens: Some(1550),
            drift_ratio: Some(0.033),
            call_count: 5,
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: CostSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn cost_summary_row_for_finds_model() {
        let row = ModelCostRow {
            model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024"),
            cost_cents: 50.0,
            call_count: 2,
            client_estimated_tokens: 100,
            provider_reported_tokens: None,
        };
        let s = CostSummary {
            by_model: vec![row.clone()],
            total_cents: 50.0,
            total_client_estimated_tokens: 100,
            total_provider_reported_tokens: None,
            drift_ratio: None,
            call_count: 2,
        };
        assert_eq!(
            s.row_for(&ModelIdentity::hosted("openai", "gpt-4", "2024")),
            Some(&row)
        );
        assert_eq!(
            s.row_for(&ModelIdentity::hosted("openai", "gpt-3.5", "2024")),
            None
        );
    }

    #[test]
    fn model_hash_mismatch_event_serde() {
        let e = ModelHashMismatchEvent {
            event_id: Uuid::new_v4(),
            tenant_id: TenantId::new(),
            model_identity: ModelIdentity::hosted("p", "m", "v"),
            expected_hash: Blake3Hash::from_bytes([0xAAu8; 32]),
            actual_hash: Blake3Hash::from_bytes([0xBBu8; 32]),
            detected_at: SystemTime::now(),
            trace_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&e).unwrap();
        let e2: ModelHashMismatchEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, e2);
    }
}
