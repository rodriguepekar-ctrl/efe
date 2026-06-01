//! PR #22 § 12 — public site + badges + JSON API.
//!
//! Phase-2 implementation :
//!   - **mdbook generation** : shells out to `mdbook` if installed ;
//!     otherwise skips with a warning.
//!   - **Badges** : 4 SVGs written under `<output>/badges/`.
//!   - **JSON API** : `<output>/current.json` + per-month/quarter.
//!   - **No client-side JS** (T100 mitigation — AC26).

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use tracing::{info, warn};

use crate::cli::PublishArgs;
use crate::error::{ExitCode, SloError, SloResult};
use crate::sli::SliVerdict;
use crate::storage;
use crate::SloId;

pub async fn run(args: PublishArgs) -> SloResult<ExitCode> {
    let chart_version = args
        .chart_version
        .as_deref()
        .ok_or_else(|| SloError::Config("--chart-version required".into()))?;
    info!(output = %args.output.display(), chart_version = %chart_version, "publish starting");

    std::fs::create_dir_all(&args.output).map_err(|source| SloError::StorageIo {
        path: args.output.clone(),
        source,
    })?;
    std::fs::create_dir_all(args.output.join("badges")).map_err(|source| SloError::StorageIo {
        path: args.output.clone(),
        source,
    })?;
    std::fs::create_dir_all(args.output.join("history")).map_err(|source| SloError::StorageIo {
        path: args.output.clone(),
        source,
    })?;
    std::fs::create_dir_all(args.output.join("quarterly")).map_err(|source| SloError::StorageIo {
        path: args.output.clone(),
        source,
    })?;

    // ── 1.  Read the latest monthly file to build /current.json + badges.
    let latest_monthly = find_latest_monthly(&args.storage_root, chart_version)?;
    let (current_payload, badge_inputs) = match latest_monthly {
        Some(m) => {
            let mut payload = current_json_payload(&m);
            payload.chart_version = chart_version.to_string();
            let mut inputs = Vec::new();
            for id in SloId::all() {
                let entry = m.slos.get(id.as_str());
                let (v, verdict) = match entry {
                    Some(e) if e.achieved.is_nan() => (f64::NAN, SliVerdict::Indeterminate),
                    Some(e) => {
                        let verdict = if e.achieved >= e.target { SliVerdict::Pass } else { SliVerdict::Fail };
                        (e.achieved, verdict)
                    }
                    None => (f64::NAN, SliVerdict::Indeterminate),
                };
                inputs.push((id, v, verdict));
            }
            (payload, inputs)
        }
        None => (
            CurrentJson::empty(chart_version),
            SloId::all().into_iter().map(|id| (id, f64::NAN, SliVerdict::Indeterminate)).collect(),
        ),
    };

    // ── 2.  Write /current.json
    let current_path = args.output.join("current.json");
    std::fs::write(&current_path, serde_json::to_vec_pretty(&current_payload)?)
        .map_err(|source| SloError::StorageIo { path: current_path.clone(), source })?;
    info!(path = %current_path.display(), "current.json written");

    // ── 3.  Write 4 badges.
    for (id, value, verdict) in &badge_inputs {
        let svg = crate::badge::render(*id, *value, *verdict);
        let path = args.output.join("badges").join(crate::badge::filename(*id));
        std::fs::write(&path, svg).map_err(|source| SloError::StorageIo {
            path: path.clone(),
            source,
        })?;
    }
    info!("4 SVG badges written");

    // ── 4.  Copy monthly history files.
    copy_monthly_history(&args.storage_root, chart_version, &args.output)?;

    // ── 5.  Copy quarterly artefacts.
    copy_quarterly_archive(&args.storage_root, chart_version, &args.output)?;

    // ── 6.  Write a /version.json with build metadata.
    let v = serde_json::json!({
        "harness_version": crate::build::VERSION,
        "git_sha": crate::build::GIT_SHA,
        "stability_contract": "1.4",
    });
    std::fs::write(args.output.join("version.json"), serde_json::to_vec_pretty(&v)?)
        .map_err(|source| SloError::StorageIo { path: args.output.clone(), source })?;

    // ── 7.  mdbook (best-effort).
    if args.source.exists() {
        match Command::new("mdbook")
            .arg("build")
            .arg(&args.source)
            .arg("--dest-dir")
            .arg(&args.output)
            .output()
        {
            Ok(out) if out.status.success() => info!("mdbook build ok"),
            Ok(out) => warn!("mdbook build failed : {}", String::from_utf8_lossy(&out.stderr)),
            Err(e) => warn!("mdbook not installed : {e} — skipping site build"),
        }
    } else {
        info!(source = %args.source.display(), "no mdbook source — skipping site build");
    }

    Ok(ExitCode::Success)
}

fn find_latest_monthly(root: &Path, chart_version: &str) -> SloResult<Option<storage::json::MonthlyAggregate>> {
    let monthly_dir = root.join(format!("v{chart_version}")).join("monthly");
    if !monthly_dir.exists() {
        return Ok(None);
    }
    let pattern = monthly_dir.join("*.json");
    let mut files: Vec<_> = glob::glob(&pattern.to_string_lossy())
        .map_err(|e| SloError::Config(format!("glob : {e}")))?
        .filter_map(Result::ok)
        .collect();
    files.sort();
    if let Some(latest) = files.last() {
        return Ok(Some(storage::json::read_monthly(latest)?));
    }
    Ok(None)
}

fn copy_monthly_history(root: &Path, chart_version: &str, output: &Path) -> SloResult<()> {
    let monthly_dir = root.join(format!("v{chart_version}")).join("monthly");
    if !monthly_dir.exists() {
        return Ok(());
    }
    let pattern = monthly_dir.join("*.json");
    for entry in glob::glob(&pattern.to_string_lossy())
        .map_err(|e| SloError::Config(format!("glob : {e}")))?
        .filter_map(Result::ok)
    {
        let dst = output.join("history").join(entry.file_name().unwrap());
        std::fs::copy(&entry, &dst).map_err(|source| SloError::StorageIo {
            path: dst.clone(),
            source,
        })?;
    }
    Ok(())
}

fn copy_quarterly_archive(root: &Path, chart_version: &str, output: &Path) -> SloResult<()> {
    let q_dir = root.join(format!("v{chart_version}")).join("quarterly");
    if !q_dir.exists() {
        return Ok(());
    }
    let pattern = q_dir.join("*.*");
    for entry in glob::glob(&pattern.to_string_lossy())
        .map_err(|e| SloError::Config(format!("glob : {e}")))?
        .filter_map(Result::ok)
    {
        let dst = output.join("quarterly").join(entry.file_name().unwrap());
        std::fs::copy(&entry, &dst).map_err(|source| SloError::StorageIo {
            path: dst.clone(),
            source,
        })?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct CurrentJson {
    schema_version: u32,
    as_of: String,
    chart_version: String,
    slos: serde_json::Map<String, serde_json::Value>,
}

impl CurrentJson {
    fn empty(chart_version: &str) -> Self {
        Self {
            schema_version: 1,
            as_of: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            chart_version: chart_version.into(),
            slos: serde_json::Map::new(),
        }
    }
}

fn current_json_payload(m: &storage::json::MonthlyAggregate) -> CurrentJson {
    let mut slos = serde_json::Map::new();
    for (k, v) in &m.slos {
        // D3 fix : arch doc § 12.2 mandates `error_budget_remaining_pct`
        // on the public surface ; we ALSO carry `consumed_pct` for
        // operator convenience (both saturate at 0..1000% to handle
        // SLOs that overshot their budget).
        let remaining = (100.0 - v.error_budget_consumed_pct).clamp(0.0, 100.0);
        slos.insert(k.clone(), serde_json::json!({
            "value": v.achieved,
            "target": v.target,
            "verdict": v.verdict,
            "error_budget_consumed_pct": v.error_budget_consumed_pct,
            "error_budget_remaining_pct": remaining,
        }));
    }
    CurrentJson {
        schema_version: 1,
        as_of: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        chart_version: m.chart_version.clone(),
        slos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::PublishArgs;

    #[tokio::test]
    async fn publish_writes_current_and_badges_even_without_monthly() {
        let tmp = tempfile::tempdir().unwrap();
        let args = PublishArgs {
            source: tmp.path().join("site"),  // doesn't exist
            output: tmp.path().join("book"),
            storage_root: tmp.path().to_path_buf(),
            chart_version: Some("0.6.0".into()),
        };
        let rc = run(args.clone()).await.unwrap();
        assert_eq!(rc, ExitCode::Success);
        assert!(args.output.join("current.json").exists());
        for id in SloId::all() {
            assert!(args.output.join("badges").join(crate::badge::filename(id)).exists());
        }
        assert!(args.output.join("version.json").exists());
    }

    #[tokio::test]
    async fn publish_requires_chart_version() {
        let tmp = tempfile::tempdir().unwrap();
        let args = PublishArgs {
            source: tmp.path().join("s"),
            output: tmp.path().join("o"),
            storage_root: tmp.path().to_path_buf(),
            chart_version: None,
        };
        let e = run(args).await.unwrap_err();
        assert!(matches!(e, SloError::Config(_)));
    }
}
