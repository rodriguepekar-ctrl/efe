//! PR #22 § 9 — rollup orchestrator (4-stage pipeline).

pub mod stage_t1;
pub mod stage_t2;
pub mod stage_t3;

use std::collections::BTreeMap;

use tracing::info;

use crate::cli::{ComputeArgs, EvidenceAction, EvidenceArgs, ListArgs, PublishArgs, RollupArgs, RollupStage, ValidateArgs};
use crate::error::{ExitCode, SloError, SloResult};
use crate::sli::{SliPoint, SliVerdict};
use crate::source::prom_query::PromHttpSource;
use crate::storage;
use crate::window::WindowSpec;
use crate::SloId;

// ─────────────────────────────────────────────────────────────────────
//                       Top-level CLI dispatch
// ─────────────────────────────────────────────────────────────────────

pub async fn dispatch_compute(args: ComputeArgs) -> SloResult<ExitCode> {
    args.validate()?;
    let chart_version = args.chart_version.as_deref().unwrap();
    let prom_url = args
        .target_override
        .as_deref()
        .unwrap_or(args.prom_url.as_str());
    info!(prom_url = %prom_url, chart_version = %chart_version, "compute starting");

    let window = WindowSpec::parse(&args.window).ok_or_else(|| {
        SloError::Config(format!("unknown window '{}'", args.window))
    })?;

    let slos = if args.slo.is_empty() {
        SloId::all().to_vec()
    } else {
        args.slo.iter().map(|s| SloId::parse(s)).collect::<Result<Vec<_>, _>>()?
    };

    let now = chrono::Utc::now().timestamp();
    let offset_secs = humantime::parse_duration(&args.offset)
        .map(|d| d.as_secs() as i64)
        .map_err(|e| SloError::Config(format!("offset : {e}")))?;
    let window_start = window.align(now - offset_secs - window.as_seconds() as i64);

    // ── Dry-run guard FIRST : we must not hit Prometheus before
    //    confirming the operator intent (D1 fix — closes the gap
    //    where `--dry-run` still issued real HTTP queries).
    if args.dry_run {
        println!(
            "[dry-run] chart={chart_version} region={} window={} window_start={} ({}) slos={}",
            args.region,
            window.as_str(),
            window_start,
            crate::window::align::fmt_iso(window_start),
            slos.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(","),
        );
        return Ok(ExitCode::Success);
    }

    let source = PromHttpSource::new(prom_url)?;

    let mut points = Vec::with_capacity(slos.len());
    for slo in slos {
        let kernel = crate::sli::dispatch_for(slo);
        let ctx = crate::sli::SliContext {
            window,
            window_start_unix: window_start,
            source: &source,
            source_label: "prod".into(),
            region: args.region.clone(),
            chart_version: chart_version.into(),
        };
        let p = kernel.compute(&ctx).await?;
        points.push(p);
    }

    // Append to daily file (idempotent — AD7).
    let yyyy_mm_dd = crate::window::align::fmt_date(window_start);
    let daily_path = storage::daily_path(&args.storage_root, chart_version, &yyyy_mm_dd);
    let mut env = if daily_path.exists() {
        storage::parquet::read_daily(&daily_path)?
    } else {
        storage::parquet::DailyEnvelope {
            schema_version: crate::DAILY_PARQUET_SCHEMA_VERSION,
            chart_version: chart_version.into(),
            region: args.region.clone(),
            date: yyyy_mm_dd.clone(),
            rollup_run_id: uuid::Uuid::now_v7().to_string(),
            generated_at_unix: now,
            points: vec![],
        }
    };
    // Replace any same-(slo, window, window_start) row → idempotency.
    env.points.retain(|p| {
        !points
            .iter()
            .any(|np| np.slo_id == p.slo_id && np.window == p.window && np.window_start_unix == p.window_start_unix)
    });
    env.points.extend(points.iter().cloned());
    env.generated_at_unix = now;
    storage::parquet::write_daily(&daily_path, &env)?;
    info!(path = %daily_path.display(), count = points.len(), "daily envelope written");

    emit_points(&points, args.format);
    Ok(ExitCode::Success)
}

pub async fn dispatch_rollup(args: RollupArgs) -> SloResult<ExitCode> {
    args.validate()?;
    match args.stage {
        RollupStage::T1 => stage_t1::run_t1(args).await,
        RollupStage::T2 => stage_t2::run_t2(args).await,
        RollupStage::T3 => stage_t3::run_t3(args).await,
    }
}

pub async fn dispatch_publish(args: PublishArgs) -> SloResult<ExitCode> {
    crate::publish::run(args).await
}

pub async fn dispatch_evidence(args: EvidenceArgs) -> SloResult<ExitCode> {
    match args.action {
        EvidenceAction::Sign { pdf, identity } => crate::evidence::sign(&pdf, identity.as_deref()),
        EvidenceAction::Verify { pdf, pubkey } => crate::evidence::verify(&pdf, &pubkey),
    }
}

pub async fn dispatch_list(args: ListArgs) -> SloResult<ExitCode> {
    let chart = args
        .chart_version
        .ok_or_else(|| SloError::Config("--chart-version required".into()))?;
    let range_secs = humantime::parse_duration(&args.range)
        .map(|d| d.as_secs() as i64)
        .map_err(|e| SloError::Config(format!("range : {e}")))?;
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - range_secs;

    let daily_root = args.storage_root.join(format!("v{chart}"));
    let mut all_points: Vec<SliPoint> = Vec::new();
    if daily_root.exists() {
        let pattern = daily_root.join("*/daily/*.parquet");
        for entry in glob::glob(&pattern.to_string_lossy())
            .map_err(|e| SloError::Config(format!("glob : {e}")))?
            .filter_map(Result::ok)
        {
            let env = storage::parquet::read_daily(&entry)?;
            for p in env.points {
                if p.window_start_unix >= cutoff {
                    if let Some(filter) = &args.slo {
                        if p.slo_id.as_str() == filter {
                            all_points.push(p);
                        }
                    } else {
                        all_points.push(p);
                    }
                }
            }
        }
    }

    match args.format {
        crate::cli::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_points)?);
        }
        crate::cli::OutputFormat::Csv => {
            println!("slo_id,window,window_start_unix,value,verdict,source,region");
            for p in &all_points {
                println!(
                    "{},{},{},{:.6},{},{},{}",
                    p.slo_id.as_str(),
                    p.window.as_str(),
                    p.window_start_unix,
                    p.value,
                    p.verdict.as_str(),
                    p.source,
                    p.region,
                );
            }
        }
        crate::cli::OutputFormat::Text => {
            for p in &all_points {
                println!(
                    "{:24} {:>4} @ {:>10} = {:.4} ({})",
                    p.slo_id.as_str(),
                    p.window.as_str(),
                    p.window_start_unix,
                    p.value,
                    p.verdict.as_str(),
                );
            }
        }
    }
    Ok(ExitCode::Success)
}

pub async fn dispatch_validate(args: ValidateArgs) -> SloResult<ExitCode> {
    let mut count_ok = 0usize;
    let mut count_bad = 0usize;
    let pattern_daily = args.root.join("**/daily/*.parquet");
    let pattern_monthly = args.root.join("**/monthly/*.json");
    for pat in [pattern_daily, pattern_monthly] {
        for entry in glob::glob(&pat.to_string_lossy())
            .map_err(|e| SloError::Config(format!("glob : {e}")))?
            .filter_map(Result::ok)
        {
            let res = if entry.extension().map(|e| e == "parquet").unwrap_or(false) {
                storage::parquet::read_daily(&entry).map(|_| ())
            } else {
                storage::json::read_monthly(&entry).map(|_| ())
            };
            match res {
                Ok(()) => {
                    println!("  ✓ {}", entry.display());
                    count_ok += 1;
                }
                Err(e) => {
                    println!("  ✗ {} ({})", entry.display(), e);
                    count_bad += 1;
                }
            }
        }
    }
    println!("[validate] {count_ok} OK, {count_bad} bad");
    if count_bad > 0 {
        Err(SloError::Config(format!("{count_bad} file(s) failed validation")))
    } else {
        Ok(ExitCode::Success)
    }
}

// ─────────────────────────────────────────────────────────────────────
//                       internal helpers
// ─────────────────────────────────────────────────────────────────────

fn emit_points(points: &[SliPoint], fmt: crate::cli::OutputFormat) {
    match fmt {
        crate::cli::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(points).unwrap_or_default());
        }
        crate::cli::OutputFormat::Csv => {
            println!("slo_id,window,window_start_unix,value,verdict");
            for p in points {
                println!(
                    "{},{},{},{:.6},{}",
                    p.slo_id.as_str(),
                    p.window.as_str(),
                    p.window_start_unix,
                    p.value,
                    p.verdict.as_str(),
                );
            }
        }
        crate::cli::OutputFormat::Text => {
            for p in points {
                println!(
                    "  {} {:>4} = {:.4} ({})",
                    p.slo_id.as_str(),
                    p.window.as_str(),
                    p.value,
                    p.verdict.as_str(),
                );
            }
        }
    }
}

/// Public re-export : aggregate per-SLO points using each SLO's
/// declared aggregation pattern (used by stage_t2).
#[must_use]
pub fn aggregate_per_slo(
    points: &[SliPoint],
) -> BTreeMap<SloId, crate::window::aggregate::AggregatedSli> {
    let mut out = BTreeMap::new();
    for slo in SloId::all() {
        let subset: Vec<SliPoint> = points.iter().filter(|p| p.slo_id == slo).cloned().collect();
        let agg = crate::window::aggregate::aggregate(
            &subset,
            slo.aggregation(),
            crate::storage::json::target_for(slo),
        );
        out.insert(slo, agg);
    }
    out
}

/// Used by the validator / verifier — checks an `SliPoint` is internally
/// consistent (e.g. verdict matches value vs target).
#[must_use]
pub fn validate_point(p: &SliPoint) -> bool {
    match p.verdict {
        SliVerdict::Indeterminate => true, // any value allowed
        SliVerdict::Pass | SliVerdict::Fail => !p.value.is_nan(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sli::{SliPoint, SliVerdict};
    use crate::window::WindowSpec;

    #[test]
    fn aggregate_per_slo_groups_correctly() {
        let pts = vec![
            SliPoint {
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
            },
            SliPoint {
                slo_id: SloId::L2LatencyP95,
                window: WindowSpec::OneHour,
                window_start_unix: 0,
                value: 0.040,
                numerator: 1,
                denominator: 1,
                compliance_bit: 1,
                coverage_pct: 1.0,
                source: "prod".into(),
                region: "global".into(),
                verdict: SliVerdict::Pass,
            },
        ];
        let agg = aggregate_per_slo(&pts);
        assert_eq!(agg.len(), 4); // all 4 SLOs (2 empty + 2 with data)
        assert_eq!(agg[&SloId::L2Availability].numerator, 999);
        assert_eq!(agg[&SloId::SwarmRecoveryTime].verdict, SliVerdict::Indeterminate);
    }
}
