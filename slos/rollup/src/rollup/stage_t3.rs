//! PR #22 § 9.4 — stage T3 (quarterly signed PDF + JSON).
//!
//! On the 1st of {Apr, Jul, Oct, Jan} @ 04:00 UTC : list the 3 monthly
//! JSONs of the previous quarter, build quarterly JSON + PDF, embed
//! SHA256, sign with cosign keyless OIDC (or skip if `--cosign-disabled`).

use std::collections::BTreeMap;

use tracing::info;

use crate::cli::RollupArgs;
use crate::error::{ExitCode, SloError, SloResult};
use crate::sli::SliVerdict;
use crate::storage;
use crate::SloId;

pub async fn run_t3(args: RollupArgs) -> SloResult<ExitCode> {
    let chart_version = args
        .chart_version
        .as_deref()
        .ok_or_else(|| SloError::Config("--chart-version required".into()))?;
    let quarter = &args.period;
    info!(quarter = %quarter, chart_version = %chart_version, "T3 starting");

    let quarter_start = crate::window::align::parse_quarter_to_unix(quarter)
        .ok_or_else(|| SloError::Config(format!("bad quarter '{quarter}'")))?;

    // ── 1.  Identify the 3 months of the quarter.
    let months = months_in_quarter(quarter_start);
    info!(months = ?months, "quarter month layout");

    // ── 2.  Read all 3 monthly JSONs.  Fail (AC16) if any is missing.
    let mut monthlies = Vec::with_capacity(3);
    for m in &months {
        let path = storage::monthly_path(&args.storage_root, chart_version, m);
        if !path.exists() {
            return Err(SloError::StorageIo {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "monthly file missing"),
            });
        }
        monthlies.push(storage::json::read_monthly(&path)?);
    }

    // ── 3.  Aggregate.
    let agg = aggregate_quarterly(&monthlies);

    // ── 4.  Build quarterly JSON.
    let quarterly_json = build_quarterly_json(chart_version, &args.region, quarter, &agg, &months);

    // ── 5.  Build PDF.
    let achievements: Vec<_> = agg
        .iter()
        .map(|(slo, value)| {
            let target = crate::storage::json::target_for(*slo);
            let verdict = if value.is_nan() {
                "indeterminate"
            } else if *value >= target {
                "pass"
            } else {
                "fail"
            };
            (slo.as_str().into(), target, *value, verdict)
        })
        .collect();
    let body = crate::storage::pdf::render_quarterly_body(
        quarter,
        chart_version,
        &achievements,
        "https://efe.github.io/slos/methodology.html",
        "audit-trail-via-rekor",
    );
    let json_sidecar = serde_json::to_vec_pretty(&quarterly_json)?;
    let pdf_bytes = crate::storage::pdf::build_quarterly_pdf(
        &format!("EFE SLO Evidence {quarter}"),
        &body,
        &json_sidecar,
    )?;

    let pdf_path = storage::quarterly_path(&args.storage_root, chart_version, quarter, "pdf");
    let json_path = storage::quarterly_path(&args.storage_root, chart_version, quarter, "json");

    if args.dry_run {
        println!("[T3] (dry-run) would write {}", pdf_path.display());
        println!("[T3] (dry-run) would write {}", json_path.display());
        return Ok(ExitCode::Success);
    }

    // Write JSON sidecar first (so the PDF SHA matches the on-disk JSON).
    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SloError::StorageIo {
            path: json_path.clone(),
            source,
        })?;
    }
    std::fs::write(&json_path, &json_sidecar).map_err(|source| SloError::StorageIo {
        path: json_path.clone(),
        source,
    })?;
    crate::storage::pdf::write_pdf(&pdf_path, &pdf_bytes)?;
    info!(pdf = %pdf_path.display(), json = %json_path.display(), "quarterly artefacts written");

    if !args.cosign_disabled {
        if let Some(identity) = &args.cosign_identity {
            crate::evidence::sign(&pdf_path, Some(identity))?;
        } else {
            return Err(SloError::Config(
                "T3 with cosign enabled requires --cosign-identity".into(),
            ));
        }
    }

    Ok(ExitCode::Success)
}

fn months_in_quarter(quarter_start_unix: i64) -> Vec<String> {
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(quarter_start_unix, 0).unwrap();
    let y = dt.naive_utc().date().format("%Y").to_string();
    let first_m = match crate::window::align::quarter_of(quarter_start_unix) {
        1 => 1,
        2 => 4,
        3 => 7,
        4 => 10,
        _ => 1,
    };
    (0..3)
        .map(|i| format!("{y}-{:02}", first_m + i))
        .collect()
}

fn aggregate_quarterly(monthlies: &[storage::json::MonthlyAggregate]) -> BTreeMap<SloId, f64> {
    let mut out = BTreeMap::new();
    for slo in SloId::all() {
        // For ratio-of-sums (SLO-1/3) : sum numerators + sum denominators.
        // For window-compliance (SLO-2/4) : mean of monthly achieved
        // values (acceptable Phase-2 approximation ; Phase-3 G.11
        // re-projects from daily for true precision).
        let key = slo.as_str();
        match slo.aggregation() {
            crate::SliAggregation::RatioOfSums => {
                let mut num = 0u64;
                let mut den = 0u64;
                for m in monthlies {
                    if let Some(e) = m.slos.get(key) {
                        num += e.numerator;
                        den += e.denominator;
                    }
                }
                let v = if den == 0 { f64::NAN } else { num as f64 / den as f64 };
                out.insert(slo, v);
            }
            crate::SliAggregation::WindowCompliance => {
                let vals: Vec<f64> = monthlies
                    .iter()
                    .filter_map(|m| m.slos.get(key).map(|e| e.achieved))
                    .filter(|v| !v.is_nan())
                    .collect();
                let v = if vals.is_empty() {
                    f64::NAN
                } else {
                    vals.iter().sum::<f64>() / vals.len() as f64
                };
                out.insert(slo, v);
            }
        }
    }
    out
}

fn build_quarterly_json(
    chart_version: &str,
    region: &str,
    quarter: &str,
    agg: &BTreeMap<SloId, f64>,
    months: &[String],
) -> serde_json::Value {
    let mut slos = serde_json::Map::new();
    for (slo, v) in agg {
        let target = crate::storage::json::target_for(*slo);
        let verdict = if v.is_nan() {
            SliVerdict::Indeterminate
        } else if *v >= target {
            SliVerdict::Pass
        } else {
            SliVerdict::Fail
        };
        slos.insert(
            slo.as_str().into(),
            serde_json::json!({
                "target": target,
                "achieved": v,
                "verdict": verdict.as_str(),
            }),
        );
    }
    serde_json::json!({
        "schema_version": crate::QUARTERLY_EVIDENCE_SCHEMA_VERSION,
        "chart_version": chart_version,
        "region": region,
        "quarter": quarter,
        "computed_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "months": months,
        "slos": slos,
        "stability_contract_version": "1.4",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::json::MonthlySloEntry;
    use std::collections::BTreeMap;

    fn make_monthly(period: &str, achieved: f64) -> storage::json::MonthlyAggregate {
        let mut slos = BTreeMap::new();
        for id in SloId::all() {
            slos.insert(id.as_str().into(), MonthlySloEntry {
                target: crate::storage::json::target_for(id),
                achieved,
                verdict: if achieved >= 0.995 { "pass" } else { "fail" }.into(),
                numerator: (achieved * 1_000_000.0) as u64,
                denominator: 1_000_000,
                error_budget_consumed_pct: 50.0,
            });
        }
        storage::json::MonthlyAggregate {
            schema_version: 1,
            chart_version: "0.6.0".into(),
            region: "global".into(),
            period: period.into(),
            computed_at: "2026-06-01T00:00:00Z".into(),
            rollup_run_id: "r".into(),
            slos,
            missing_days: vec![],
            stability_contract_version: "1.4".into(),
        }
    }

    #[test]
    fn months_in_quarter_q2_2026() {
        let ts = crate::window::align::parse_quarter_to_unix("Q2-2026").unwrap();
        let m = months_in_quarter(ts);
        assert_eq!(m, vec!["2026-04", "2026-05", "2026-06"]);
    }

    #[test]
    fn months_in_quarter_q4_2026() {
        let ts = crate::window::align::parse_quarter_to_unix("Q4-2026").unwrap();
        let m = months_in_quarter(ts);
        assert_eq!(m, vec!["2026-10", "2026-11", "2026-12"]);
    }

    #[test]
    fn aggregate_quarterly_combines_3_months() {
        let monthlies = vec![
            make_monthly("2026-04", 0.998),
            make_monthly("2026-05", 0.999),
            make_monthly("2026-06", 0.997),
        ];
        let agg = aggregate_quarterly(&monthlies);
        // RatioOfSums : sum / sum
        let avail = agg[&SloId::L2Availability];
        // 998_000 + 999_000 + 997_000 = 2_994_000 / 3_000_000 = 0.998
        assert!((avail - 0.998).abs() < 1e-9);
    }

    #[test]
    fn quarterly_json_lists_3_months() {
        let monthlies = vec![
            make_monthly("2026-04", 0.998),
            make_monthly("2026-05", 0.999),
            make_monthly("2026-06", 0.997),
        ];
        let agg = aggregate_quarterly(&monthlies);
        let q = build_quarterly_json(
            "0.6.0", "global", "Q2-2026", &agg,
            &["2026-04".into(), "2026-05".into(), "2026-06".into()],
        );
        assert_eq!(q["quarter"], "Q2-2026");
        assert_eq!(q["months"].as_array().unwrap().len(), 3);
        assert_eq!(q["stability_contract_version"], "1.4");
    }
}
