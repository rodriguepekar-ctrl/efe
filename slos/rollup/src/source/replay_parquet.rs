//! PR #22 § 14 — replay a stored daily file (for debugging + tests).
//!
//! Phase-2 uses the JSON-backed daily store (see `storage::json` ;
//! Parquet is implemented in the same backend abstraction with a JSON
//! envelope wrapper for portability).  Phase-3 will swap in a true
//! Parquet writer (G.9 schema-migration window).

use std::path::Path;

use async_trait::async_trait;

use crate::error::{SloError, SloResult};
use crate::sli::SliPoint;
use crate::source::{EventQuery, HistogramQuery, MetricQuery, RecoveryEvent, SourceProvider};

pub struct ReplaySource {
    points: Vec<SliPoint>,
}

impl ReplaySource {
    /// Load a stored daily file (JSON-envelope Parquet — see § 10.1
    /// for the future binary Parquet path).
    pub fn load(path: &Path) -> SloResult<Self> {
        let raw = std::fs::read(path).map_err(|source| SloError::StorageIo {
            path: path.to_path_buf(),
            source,
        })?;
        let points: Vec<SliPoint> = serde_json::from_slice(&raw)?;
        Ok(Self { points })
    }
}

#[async_trait]
impl SourceProvider for ReplaySource {
    async fn integer_counter(&self, _q: MetricQuery) -> SloResult<u64> {
        Ok(self.points.iter().map(|p| p.numerator).sum())
    }
    async fn histogram_quantile(&self, _q: HistogramQuery) -> SloResult<f64> {
        // Average across stored points — debugging-grade only.
        let vals: Vec<f64> = self.points.iter().map(|p| p.value).filter(|v| !v.is_nan()).collect();
        if vals.is_empty() {
            return Ok(f64::NAN);
        }
        Ok(vals.iter().sum::<f64>() / vals.len() as f64)
    }
    async fn recovery_events(&self, _q: EventQuery) -> SloResult<Vec<RecoveryEvent>> {
        Ok(vec![])
    }
    async fn scrape_coverage(&self, _: i64, _: u64) -> SloResult<f64> {
        Ok(1.0)
    }
    async fn participants(&self, _: i64, _: u64) -> SloResult<Vec<String>> {
        Ok(vec![])
    }
}
