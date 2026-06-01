//! PR #22 § 10 + AD27 — retention policy.
//!
//!   - daily   : 90 days
//!   - monthly : 7 years
//!   - quarterly : forever

use std::path::{Path, PathBuf};

use crate::error::{SloError, SloResult};

/// Returns the daily file paths that are older than `retain_days` from `now`.
pub fn expired_daily_files(root: &Path, chart_version: &str, retain_days: u32, now: chrono::DateTime<chrono::Utc>) -> SloResult<Vec<PathBuf>> {
    let daily_root = root.join(format!("v{chart_version}"));
    let cutoff = now - chrono::Duration::days(i64::from(retain_days));
    let mut out = Vec::new();
    if !daily_root.exists() {
        return Ok(out);
    }
    let pattern = daily_root.join("*/daily/*.parquet");
    let pattern_str = pattern.to_string_lossy().to_string();
    for entry in glob::glob(&pattern_str)
        .map_err(|e| SloError::Config(format!("glob : {e}")))?
        .filter_map(Result::ok)
    {
        // Filename : YYYY-MM-DD.parquet
        let stem = entry.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        if let Ok(d) = chrono::NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
            let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                d.and_hms_opt(0, 0, 0).unwrap(),
                chrono::Utc,
            );
            if dt < cutoff {
                out.push(entry);
            }
        }
    }
    Ok(out)
}

/// Default daily retention (days).
pub const DAILY_RETAIN_DAYS: u32 = 90;
/// Default monthly retention (days).
pub const MONTHLY_RETAIN_DAYS: u32 = 7 * 365;
/// Quarterly retention : effectively forever.
pub const QUARTERLY_RETAIN_DAYS: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn expired_daily_finds_old_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let daily_dir = root.join("v0.6.0/2026-01/daily");
        fs::create_dir_all(&daily_dir).unwrap();
        // Old file (180 days back)
        fs::write(daily_dir.join("2026-01-01.parquet"), "{}").unwrap();
        // Recent file (5 days back)
        fs::write(daily_dir.join("2026-07-01.parquet"), "{}").unwrap();

        let now = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 6).unwrap().and_hms_opt(0, 0, 0).unwrap(),
            chrono::Utc,
        );
        let expired = expired_daily_files(root, "0.6.0", 90, now).unwrap();
        assert_eq!(expired.len(), 1);
        assert!(expired[0].file_name().unwrap() == "2026-01-01.parquet");
    }

    #[test]
    fn retention_constants_frozen() {
        // STABILITY_CONTRACT v1.4 § 22 (retention policy).
        assert_eq!(DAILY_RETAIN_DAYS, 90);
        assert_eq!(MONTHLY_RETAIN_DAYS, 2555); // 7 × 365
    }
}
