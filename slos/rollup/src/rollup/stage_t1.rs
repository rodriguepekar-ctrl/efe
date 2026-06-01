//! PR #22 § 9.2 — stage T1 (hourly daily-Parquet builder).
//!
//! Invoked by the `rollup --stage t1 --period <yyyy-mm-dd>` CLI path
//! (typically as a back-fill tool ; the in-cluster CronJob calls
//! `compute --window 1h --offset 1h` 24× per day, once per hour).
//!
//! In production T1 :
//!   1. Computes the 4 SLI points for **each of the 24 hours of that
//!      day**, across the 4 burn-rate windows.
//!   2. Writes them to the daily envelope at
//!      `<storage_root>/v{chart}/<yyyy-mm>/daily/<yyyy-mm-dd>.parquet`.
//!   3. Is idempotent (re-run for the same day with same scrape → byte-
//!      identical output ; AD7).

use tracing::{info, warn};

use crate::cli::{ComputeArgs, OutputFormat, RollupArgs};
use crate::error::{ExitCode, SloError, SloResult};
use crate::storage;
use crate::window::WindowSpec;

pub async fn run_t1(args: RollupArgs) -> SloResult<ExitCode> {
    let chart_version = args
        .chart_version
        .as_deref()
        .ok_or_else(|| SloError::Config("--chart-version required".into()))?;
    let date = &args.period;
    info!(date = %date, chart_version = %chart_version, "T1 starting");

    let date_start = crate::window::align::parse_date_to_unix(date)
        .ok_or_else(|| SloError::Config(format!("bad date '{date}' (expected yyyy-mm-dd)")))?;
    let now = chrono::Utc::now().timestamp();

    if args.dry_run {
        println!("[T1] (dry-run) chart={chart_version} date={date}");
        println!("[T1]   would compute 24 hours × 4 SLOs × 4 windows = 384 points");
        println!("[T1]   target file : {}", storage::daily_path(&args.storage_root, chart_version, date).display());
        return Ok(ExitCode::Success);
    }

    // Loop the 24 hours of the day and invoke `dispatch_compute` for each.
    let mut total_points = 0usize;
    for hour in 0..24 {
        let hour_start = date_start + (hour * 3600);
        let hour_end = hour_start + 3600;
        if hour_end > now {
            // Future hour : skip (won't have data yet).
            info!(hour, "skipping future hour");
            break;
        }
        // For each burn-rate-relevant window, invoke compute.
        for window in [WindowSpec::FiveMin, WindowSpec::ThirtyMin, WindowSpec::OneHour, WindowSpec::SixHours] {
            // Skip windows larger than the current offset (e.g. can't
            // compute a 6h window for the first hour of the day).
            if window.as_seconds() as i64 > (now - hour_start) {
                continue;
            }
            let offset_seconds = now - hour_end;
            let compute_args = ComputeArgs {
                slo: vec![],
                window: window.as_str().into(),
                offset: format!("{offset_seconds}s"),
                backfill: None,
                release_backfill_confirmed: false,
                prom_url: "http://prometheus.monitoring.svc:9090".to_string(),
                target_override: None,
                storage_root: args.storage_root.clone(),
                chart_version: Some(chart_version.into()),
                region: args.region.clone(),
                format: OutputFormat::Json,
                dry_run: false,
            };
            match super::dispatch_compute(compute_args).await {
                Ok(_) => total_points += 4, // 4 SLOs per call
                Err(e) => {
                    warn!(error = %e, hour, ?window, "skipping window");
                }
            }
        }
    }
    info!(total_points, "T1 complete");
    Ok(ExitCode::Success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RollupStage;

    #[tokio::test]
    async fn t1_dry_run_succeeds() {
        let args = RollupArgs {
            stage: RollupStage::T1,
            period: "2026-05-15".into(),
            chart_version: Some("0.6.0".into()),
            region: "global".into(),
            storage_root: std::env::temp_dir(),
            cosign_identity: None,
            cosign_disabled: true,
            dry_run: true,
        };
        let rc = run_t1(args).await.unwrap();
        assert_eq!(rc, ExitCode::Success);
    }

    #[tokio::test]
    async fn t1_requires_chart_version() {
        let args = RollupArgs {
            stage: RollupStage::T1,
            period: "2026-05-15".into(),
            chart_version: None,
            region: "global".into(),
            storage_root: std::env::temp_dir(),
            cosign_identity: None,
            cosign_disabled: true,
            dry_run: true,
        };
        let e = run_t1(args).await.unwrap_err();
        assert_eq!(e.exit_code(), ExitCode::Config);
    }

    #[tokio::test]
    async fn t1_rejects_bad_date() {
        let args = RollupArgs {
            stage: RollupStage::T1,
            period: "not-a-date".into(),
            chart_version: Some("0.6.0".into()),
            region: "global".into(),
            storage_root: std::env::temp_dir(),
            cosign_identity: None,
            cosign_disabled: true,
            dry_run: false,
        };
        let e = run_t1(args).await.unwrap_err();
        assert!(matches!(e, SloError::Config(_)));
    }
}
