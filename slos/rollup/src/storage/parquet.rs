//! PR #22 § 10.1 — daily storage layer.
//!
//! Phase-2 implementation : JSON-envelope-encoded Parquet substitute.
//! The on-disk format is **a serde_json array of SliPoint records with
//! schema_version + chart_version headers**.  Phase-3 (Appendix G.9)
//! will introduce a true Apache Arrow Parquet writer keyed by the same
//! schema version — operators can replay either via [`ReplaySource`].
//!
//! [`ReplaySource`]: crate::source::replay_parquet::ReplaySource

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{SloError, SloResult};
use crate::sli::SliPoint;

/// On-disk envelope written by [`write_daily`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyEnvelope {
    pub schema_version: u32,
    pub chart_version: String,
    pub region: String,
    pub date: String,
    pub rollup_run_id: String,
    pub generated_at_unix: i64,
    pub points: Vec<SliPoint>,
}

impl DailyEnvelope {
    /// Validate schema version on load.
    pub fn validate(&self) -> SloResult<()> {
        if self.schema_version != crate::DAILY_PARQUET_SCHEMA_VERSION {
            return Err(SloError::SchemaVersion {
                expected: crate::DAILY_PARQUET_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        Ok(())
    }
}

/// Append a daily envelope to disk.  Atomic via temp + rename (AD29).
pub fn write_daily(path: &Path, env: &DailyEnvelope) -> SloResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SloError::StorageIo {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let body = serde_json::to_vec_pretty(env)?;
    let tmp = path.with_extension("parquet.tmp");
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

/// Read a daily envelope from disk.
pub fn read_daily(path: &Path) -> SloResult<DailyEnvelope> {
    let raw = std::fs::read(path).map_err(|source| SloError::StorageIo {
        path: path.to_path_buf(),
        source,
    })?;
    let env: DailyEnvelope = serde_json::from_slice(&raw)?;
    env.validate()?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sli::{SliPoint, SliVerdict};
    use crate::window::WindowSpec;
    use crate::SloId;

    fn sample_envelope() -> DailyEnvelope {
        DailyEnvelope {
            schema_version: 1,
            chart_version: "0.6.0".into(),
            region: "global".into(),
            date: "2026-05-15".into(),
            rollup_run_id: "r-test".into(),
            generated_at_unix: 1_779_667_200,
            points: vec![SliPoint {
                slo_id: SloId::L2Availability,
                window: WindowSpec::OneHour,
                window_start_unix: 0,
                value: 0.999,
                numerator: 999,
                denominator: 1000,
                compliance_bit: 255,
                coverage_pct: 1.0,
                source: "prod".into(),
                region: "global".into(),
                verdict: SliVerdict::Pass,
            }],
        }
    }

    #[test]
    fn write_then_read_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("2026-05-15.parquet");
        let env = sample_envelope();
        write_daily(&p, &env).unwrap();
        let back = read_daily(&p).unwrap();
        assert_eq!(back.points.len(), 1);
        assert_eq!(back.points[0].slo_id, SloId::L2Availability);
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bad.parquet");
        let mut env = sample_envelope();
        env.schema_version = 99;
        std::fs::write(&p, serde_json::to_vec(&env).unwrap()).unwrap();
        let e = read_daily(&p).unwrap_err();
        assert!(matches!(e, SloError::SchemaVersion { .. }));
    }

    #[test]
    fn atomic_rename_no_partial_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("atomic.parquet");
        write_daily(&p, &sample_envelope()).unwrap();
        // No leftover .tmp file.
        let tmp_path = p.with_extension("parquet.tmp");
        assert!(!tmp_path.exists());
    }
}
