//! PR #22 § 9.3 — stage T2 (monthly JSON builder).
//!
//! On the 1st of month @ 03:00 UTC : list every daily file for the
//! previous month, aggregate each SLO via [`aggregate_per_slo`], emit
//! a canonical-form JSON.

use chrono::Datelike;
use tracing::info;

use crate::cli::RollupArgs;
use crate::error::{ExitCode, SloError, SloResult};
use crate::storage;

pub async fn run_t2(args: RollupArgs) -> SloResult<ExitCode> {
    let chart_version = args
        .chart_version
        .as_deref()
        .ok_or_else(|| SloError::Config("--chart-version required".into()))?;
    let yyyy_mm = &args.period;
    info!(period = %yyyy_mm, chart_version = %chart_version, "T2 starting");

    // ── 1.  List daily files for the period.
    let pattern = args
        .storage_root
        .join(format!("v{chart_version}"))
        .join(yyyy_mm)
        .join("daily")
        .join("*.parquet");
    let mut daily_files: Vec<_> = glob::glob(&pattern.to_string_lossy())
        .map_err(|e| SloError::Config(format!("glob : {e}")))?
        .filter_map(Result::ok)
        .collect();
    daily_files.sort();
    info!(count = daily_files.len(), "daily files found");

    // ── 2.  Read & flatten all points.
    let mut all_points = Vec::new();
    let mut seen_dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in &daily_files {
        let env = storage::parquet::read_daily(f)?;
        seen_dates.insert(env.date.clone());
        all_points.extend(env.points);
    }

    // ── 3.  Compute missing days.
    let missing = compute_missing_days(yyyy_mm, &seen_dates)?;

    // ── 4.  Aggregate per SLO.
    let agg = super::aggregate_per_slo(&all_points);

    // ── 5.  Build monthly JSON.
    let monthly = storage::json::build_monthly(
        chart_version,
        &args.region,
        yyyy_mm,
        &uuid::Uuid::now_v7().to_string(),
        &agg,
        missing,
    );

    let path = storage::monthly_path(&args.storage_root, chart_version, yyyy_mm);
    if args.dry_run {
        println!("[T2] (dry-run) would write {}", path.display());
        println!("{}", serde_json::to_string_pretty(&monthly)?);
        return Ok(ExitCode::Success);
    }
    storage::json::write_monthly(&path, &monthly)?;
    info!(path = %path.display(), "monthly JSON written");
    Ok(ExitCode::Success)
}

/// Determine which days of `yyyy_mm` are absent from `seen_dates`.
fn compute_missing_days(yyyy_mm: &str, seen: &std::collections::BTreeSet<String>) -> SloResult<Vec<String>> {
    let first_unix = crate::window::align::parse_month_to_unix(yyyy_mm)
        .ok_or_else(|| SloError::Config(format!("bad period '{yyyy_mm}'")))?;
    let first = chrono::DateTime::<chrono::Utc>::from_timestamp(first_unix, 0).unwrap();
    let next_month = if first.naive_utc().month() == 12 {
        chrono::NaiveDate::from_ymd_opt(first.naive_utc().year() + 1, 1, 1).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(first.naive_utc().year(), first.naive_utc().month() + 1, 1).unwrap()
    };
    let last_day = next_month.pred_opt().unwrap();
    let mut missing = Vec::new();
    let mut d = first.naive_utc().date();
    while d <= last_day {
        let s = d.format("%Y-%m-%d").to_string();
        if !seen.contains(&s) {
            missing.push(s);
        }
        d = d.succ_opt().unwrap();
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn compute_missing_full_month_when_no_data() {
        let seen = BTreeSet::new();
        let missing = compute_missing_days("2026-02", &seen).unwrap();
        // Feb 2026 = 28 days
        assert_eq!(missing.len(), 28);
        assert_eq!(missing[0], "2026-02-01");
        assert_eq!(missing[27], "2026-02-28");
    }

    #[test]
    fn compute_missing_empty_when_all_present() {
        let seen: BTreeSet<String> = (1..=31).map(|d| format!("2026-01-{d:02}")).collect();
        let missing = compute_missing_days("2026-01", &seen).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn compute_missing_specific_holes() {
        let mut seen = BTreeSet::new();
        for d in [1, 2, 3, 5, 6, 7] {
            seen.insert(format!("2026-04-{d:02}"));
        }
        let missing = compute_missing_days("2026-04", &seen).unwrap();
        // April = 30 days, we provided 6, so 24 are missing
        assert_eq!(missing.len(), 24);
        assert!(missing.contains(&"2026-04-04".to_string()));
    }
}
