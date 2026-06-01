//! PR #22 § 10.2 — monthly JSON storage.
//!
//! Canonical form : sorted keys (BTreeMap), 2-space indent.  Tested via
//! golden round-trip.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{SloError, SloResult};
use crate::sli::SliVerdict;
use crate::SloId;

/// Monthly aggregate written by stage T2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthlyAggregate {
    pub schema_version: u32,
    pub chart_version: String,
    pub region: String,
    /// `2026-05`.
    pub period: String,
    pub computed_at: String,
    pub rollup_run_id: String,
    /// **Sorted** by SLO ID (canonical form).
    pub slos: BTreeMap<String, MonthlySloEntry>,
    pub missing_days: Vec<String>,
    pub stability_contract_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthlySloEntry {
    pub target: f64,
    pub achieved: f64,
    pub verdict: String,
    pub numerator: u64,
    pub denominator: u64,
    pub error_budget_consumed_pct: f64,
}

impl MonthlySloEntry {
    /// Build from an aggregated SLI and the SLO target.
    #[must_use]
    pub fn from_agg(
        agg: &crate::window::aggregate::AggregatedSli,
        target: f64,
    ) -> Self {
        let consumed = if target >= 1.0 || target <= 0.0 {
            0.0
        } else {
            // Error budget consumed (%) :  100 × (1 - SLI) / (1 - target).
            let er_budget = 1.0 - target;
            if agg.value.is_nan() || er_budget < f64::EPSILON {
                0.0
            } else {
                (((1.0 - agg.value) / er_budget) * 100.0).clamp(0.0, 1_000.0)
            }
        };
        Self {
            target,
            achieved: agg.value,
            verdict: match agg.verdict {
                SliVerdict::Pass => "pass".into(),
                SliVerdict::Fail => "fail".into(),
                SliVerdict::Indeterminate => "indeterminate".into(),
            },
            numerator: agg.numerator,
            denominator: agg.denominator,
            error_budget_consumed_pct: consumed,
        }
    }
}

/// Write a monthly JSON in canonical form (AD34).
pub fn write_monthly(path: &Path, m: &MonthlyAggregate) -> SloResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SloError::StorageIo {
            path: path.to_path_buf(),
            source,
        })?;
    }
    // Canonical form : BTreeMap orders keys naturally + serde_json
    // pretty-printer uses 2-space indent.
    let body = serde_json::to_vec_pretty(m)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|source| SloError::StorageIo {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| SloError::StorageIo {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

pub fn read_monthly(path: &Path) -> SloResult<MonthlyAggregate> {
    let raw = std::fs::read(path).map_err(|source| SloError::StorageIo {
        path: path.to_path_buf(),
        source,
    })?;
    let m: MonthlyAggregate = serde_json::from_slice(&raw)?;
    if m.schema_version != crate::MONTHLY_JSON_SCHEMA_VERSION {
        return Err(SloError::SchemaVersion {
            expected: crate::MONTHLY_JSON_SCHEMA_VERSION,
            actual: m.schema_version,
        });
    }
    Ok(m)
}

/// Build a canonical monthly payload from a vec of aggregated SLIs.
///
/// `agg_per_slo` MUST have an entry for every `SloId::all()`.
#[must_use]
pub fn build_monthly(
    chart_version: &str,
    region: &str,
    period: &str,
    rollup_run_id: &str,
    agg_per_slo: &BTreeMap<SloId, crate::window::aggregate::AggregatedSli>,
    missing_days: Vec<String>,
) -> MonthlyAggregate {
    let mut slos = BTreeMap::new();
    for id in SloId::all() {
        let agg = agg_per_slo.get(&id).cloned().unwrap_or(crate::window::aggregate::AggregatedSli {
            value: f64::NAN,
            numerator: 0,
            denominator: 0,
            valid_count: 0,
            verdict: SliVerdict::Indeterminate,
        });
        let target = target_for(id);
        slos.insert(id.as_str().into(), MonthlySloEntry::from_agg(&agg, target));
    }
    MonthlyAggregate {
        schema_version: crate::MONTHLY_JSON_SCHEMA_VERSION,
        chart_version: chart_version.into(),
        region: region.into(),
        period: period.into(),
        computed_at: chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        rollup_run_id: rollup_run_id.into(),
        slos,
        missing_days,
        stability_contract_version: "1.4".into(),
    }
}

#[must_use]
pub fn target_for(slo: SloId) -> f64 {
    use crate::thresholds::*;
    match slo {
        SloId::L2Availability => L2_AVAILABILITY_TARGET,
        SloId::L2LatencyP95 => L2_LATENCY_P95_COMPLIANCE,
        SloId::SwarmRecoveryTime => SWARM_RECOVERY_TIME_COMPLIANCE,
        SloId::ReplicationLagP99 => REPLICATION_LAG_COMPLIANCE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::aggregate::AggregatedSli;

    fn agg_pass(num: u64, den: u64) -> AggregatedSli {
        AggregatedSli {
            value: num as f64 / den as f64,
            numerator: num,
            denominator: den,
            valid_count: 1,
            verdict: SliVerdict::Pass,
        }
    }

    #[test]
    fn build_monthly_includes_all_4_slos() {
        let mut agg = BTreeMap::new();
        agg.insert(SloId::L2Availability, agg_pass(999, 1000));
        agg.insert(SloId::L2LatencyP95, agg_pass(99, 100));
        agg.insert(SloId::SwarmRecoveryTime, agg_pass(15, 15));
        agg.insert(SloId::ReplicationLagP99, agg_pass(99, 100));
        let m = build_monthly("0.6.0", "global", "2026-05", "r-test", &agg, vec![]);
        assert_eq!(m.slos.len(), 4);
        // BTreeMap → deterministic key order.
        let keys: Vec<_> = m.slos.keys().collect();
        assert_eq!(
            keys,
            ["L2_AVAILABILITY", "L2_LATENCY_P95", "REPLICATION_LAG_P99", "SWARM_RECOVERY_TIME"]
        );
    }

    #[test]
    fn write_then_read_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("2026-05.json");
        let mut agg = BTreeMap::new();
        for id in SloId::all() {
            agg.insert(id, agg_pass(999, 1000));
        }
        let m = build_monthly("0.6.0", "global", "2026-05", "r-x", &agg, vec![]);
        write_monthly(&p, &m).unwrap();
        let back = read_monthly(&p).unwrap();
        assert_eq!(back.period, "2026-05");
        assert_eq!(back.slos.len(), 4);
    }

    #[test]
    fn error_budget_consumed_above_zero() {
        let mut agg = BTreeMap::new();
        // Achieved 99.0%, target 99.5% : consumed 100% of budget
        agg.insert(SloId::L2Availability, AggregatedSli {
            value: 0.99,
            numerator: 99_000,
            denominator: 100_000,
            valid_count: 1,
            verdict: SliVerdict::Fail,
        });
        for id in [SloId::L2LatencyP95, SloId::SwarmRecoveryTime, SloId::ReplicationLagP99] {
            agg.insert(id, agg_pass(99, 100));
        }
        let m = build_monthly("0.6.0", "global", "2026-05", "r-x", &agg, vec![]);
        let entry = m.slos.get("L2_AVAILABILITY").unwrap();
        // (1 - 0.99) / (1 - 0.995) = 0.01/0.005 = 200% consumed
        assert!((entry.error_budget_consumed_pct - 200.0).abs() < 1e-9);
    }
}
