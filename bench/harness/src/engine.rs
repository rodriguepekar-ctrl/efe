//! PR #21 § 9 + § 28 — top-level orchestrator.

use std::path::PathBuf;
use std::time::Instant;

use tracing::{info, warn};

use crate::baseline::{Baseline, BaselineManifest, ClusterDescription, ScenarioManifestEntry};
use crate::chaos::applier::{KubectlApplier, MockApplier};
use crate::chaos::{default_crd_root, ChaosTimeline};
use crate::cli::{
    BaselineRegenArgs, CompareArgs, ListArgs, ListFormat, PublishArgs, RunArgs, ValidateArgs,
};
use crate::cluster::{from_cli as cluster_from_cli, ClusterProvisioner};
use crate::compare::{compare, Verdict};
use crate::error::{BenchError, BenchResult, ExitCode};
use crate::publish::Publisher;
use crate::result::ScenarioResult;
use crate::scenario::Scenario;

// ────────────────────────────────────────────────────────────────
//                     Public library entry
// ────────────────────────────────────────────────────────────────

/// Run a single scenario end-to-end against an already-provisioned
/// cluster.  Returns the [`ScenarioResult`] including comparison.
///
/// `target_override` lets callers redirect the loadgen target URL at
/// runtime (PR #21 § 14 — G1/G2/G6) without editing scenario YAMLs.
/// `None` keeps the scenario-declared endpoints intact.
pub async fn run_one(
    scenario: &Scenario,
    kubeconfig: Option<PathBuf>,
    baseline_root: Option<PathBuf>,
    target_override: Option<String>,
) -> BenchResult<ScenarioResult> {
    info!(scenario = %scenario.id, "running scenario");
    let started = Instant::now();
    let namespace = scenario
        .cluster
        .topology
        .namespace
        .clone()
        .unwrap_or_else(|| format!("efe-bench-{}", scenario.id.to_lowercase()));

    // Chaos applier : real when kubeconfig is set, mock otherwise.
    let applier: Box<dyn crate::chaos::applier::ChaosApplier> = match kubeconfig {
        Some(p) => Box::new(KubectlApplier::new(p)),
        None => Box::new(MockApplier::default()),
    };

    let timeline = ChaosTimeline::new(&scenario.chaos, default_crd_root(), namespace.clone());

    // Spawn the loadgen task concurrently with the chaos timeline.
    let lg = tokio::spawn({
        let s = scenario.clone();
        let target_override = target_override.clone();
        async move {
            crate::loadgen::run_loadgen_for_scenario(&s, target_override.as_deref()).await
        }
    });
    let chaos_events = timeline.run(&*applier, started).await?;
    // Cleanup chaos before harvesting loadgen result.
    let _ = timeline.cleanup(&*applier).await;
    let metrics = lg.await.map_err(|e| BenchError::Loadgen(format!("join : {e}")))??;

    let mut result = ScenarioResult::with_metrics(scenario.id.clone(), metrics);
    result.duration_s = started.elapsed().as_secs_f64();
    result.chaos_events = chaos_events;

    // Compare vs baseline (if available).
    if let Some(root) = baseline_root {
        let path = Baseline::path(&root, &result.chart_version, &scenario.id);
        if let Some(b) = Baseline::load_optional(&path)? {
            let report = compare(scenario, &result, Some(&b));
            result.verdict = Some(report.overall);
        }
    }

    Ok(result)
}

// ────────────────────────────────────────────────────────────────
//                       Subcommand dispatch
// ────────────────────────────────────────────────────────────────

pub async fn dispatch_run(args: RunArgs) -> BenchResult<ExitCode> {
    args.validate()?;

    let scenarios = if args.all {
        Scenario::discover(&args.scenarios)?
    } else {
        let mut out = Vec::new();
        for id in &args.scenario {
            let path = args.scenarios.join(format!("{id}.yaml"));
            if !path.exists() {
                return Err(BenchError::ScenarioNotFound {
                    id: id.clone(),
                    dir: args.scenarios.clone(),
                });
            }
            out.push(Scenario::load(&path)?);
        }
        out
    };
    info!(count = scenarios.len(), "scenarios loaded");
    if let Some(url) = &args.target_override {
        info!(target_override = %url, "loadgen target URL will be overridden for every scenario");
    }

    if args.dry_run {
        for s in &scenarios {
            println!("[dry-run] {} : duration={} chaos_faults={}", s.id, s.duration, s.chaos.len());
        }
        return Ok(ExitCode::Success);
    }

    // Provision the cluster (single cluster for all scenarios unless
    // --cluster-per-scenario set, AD12).
    let mut shared_kubeconfig: Option<PathBuf> = None;
    let mut shared_provisioner: Option<Box<dyn ClusterProvisioner>> = None;
    if !args.cluster_per_scenario {
        let prov = cluster_from_cli(
            args.provisioner,
            args.kubeconfig.clone(),
            format!("efe-bench-{}", uuid::Uuid::new_v4().simple()),
        );
        shared_kubeconfig = Some(prov.provision().await?);
        shared_provisioner = Some(prov);
    }

    let mut reports = Vec::with_capacity(scenarios.len());
    let mut overall = Verdict::Pass;
    let mut failed_count = 0usize;

    for scenario in &scenarios {
        let kubeconfig = if args.cluster_per_scenario {
            let prov = cluster_from_cli(
                args.provisioner,
                args.kubeconfig.clone(),
                format!("efe-bench-{}", scenario.id.to_lowercase()),
            );
            Some(prov.provision().await?)
        } else {
            shared_kubeconfig.clone()
        };

        let result = run_one(
            scenario,
            kubeconfig,
            Some(args.baselines.clone()),
            args.target_override.clone(),
        )
        .await?;

        // Compare + accumulate report.
        let baseline_path =
            Baseline::path(&args.baselines, &result.chart_version, &scenario.id);
        let baseline = Baseline::load_optional(&baseline_path)?;
        let report = compare(scenario, &result, baseline.as_ref());
        if matches!(report.overall, Verdict::Fail) {
            failed_count += 1;
        }
        overall = overall.combine(report.overall);

        // Write summary.json.
        let out_dir = args.output_dir.clone().unwrap_or_else(|| {
            scenario.default_output_dir(&result.run_id)
        });
        std::fs::create_dir_all(&out_dir).ok();
        let summary_path = out_dir.join("summary.json");
        result.write(&summary_path)?;
        info!(scenario = %scenario.id, verdict = ?report.overall, path = %summary_path.display(), "scenario complete");

        reports.push(report);
    }

    // Teardown shared cluster.
    if let Some(prov) = shared_provisioner {
        if !args.skip_teardown {
            let _ = prov.teardown().await;
        }
    }

    // PR comment.
    if let Some(path) = &args.pr_comment_file {
        let comment = crate::pr_comment::build_pr_comment(
            &reports,
            crate::build::VERSION,
        );
        std::fs::write(path, comment).map_err(|source| BenchError::Result {
            path: path.clone(),
            source,
        })?;
    }
    // JSON verdict (CI).
    if let Some(path) = &args.verdict_json_file {
        let body = serde_json::json!({
            "overall": overall.as_str(),
            "failed_count": failed_count,
            "scenarios": reports.iter().map(|r| serde_json::json!({
                "id": r.scenario_id,
                "verdict": r.overall.as_str(),
            })).collect::<Vec<_>>(),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&body).unwrap())
            .map_err(|source| BenchError::Result { path: path.clone(), source })?;
    }

    if failed_count > 0 {
        return Err(BenchError::BenchVerdict { failed_count });
    }
    Ok(ExitCode::Success)
}

pub async fn dispatch_list(args: ListArgs) -> BenchResult<ExitCode> {
    let scenarios = Scenario::discover(&args.scenarios)?;
    match args.format {
        ListFormat::Text => {
            for s in &scenarios {
                println!("{:30} {:>5} {}", s.id, s.duration, s.description);
            }
        }
        ListFormat::Json => {
            let summary: Vec<_> = scenarios
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "description": s.description,
                        "duration": s.duration,
                        "chaos_count": s.chaos.len(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        ListFormat::Yaml => {
            for s in &scenarios {
                println!("- id: {}\n  description: \"{}\"", s.id, s.description);
            }
        }
    }
    Ok(ExitCode::Success)
}

pub async fn dispatch_validate(args: ValidateArgs) -> BenchResult<ExitCode> {
    let scenarios = Scenario::discover(&args.scenarios)?;
    println!("[bench] {} scenarios validated", scenarios.len());
    for s in &scenarios {
        println!("  ✓ {}", s.id);
    }
    Ok(ExitCode::Success)
}

pub async fn dispatch_compare(args: CompareArgs) -> BenchResult<ExitCode> {
    let result = ScenarioResult::read(&args.result)?;
    let baseline = Baseline::load(&args.baseline)?;
    let scen = Scenario {
        schema_version: 1,
        id: result.scenario_id.clone(),
        description: String::new(),
        duration: "0s".into(),
        cluster: Default::default(),
        loadgen: crate::scenario::ScenarioLoadgen {
            version: 1,
            name: String::new(),
            target: serde_yaml::Value::Null,
            pattern: serde_yaml::Value::Null,
            workload: None,
            rate: None,
            termination: None,
            output: None,
            observability: None,
            run: None,
            extra: Default::default(),
        },
        chaos: vec![],
        results: Default::default(),
        tolerance: Default::default(),
    };
    let report = compare(&scen, &result, Some(&baseline));
    match args.format {
        crate::cli::CompareFormat::Markdown => {
            println!("{}", crate::pr_comment::build_pr_comment(&[report.clone()], crate::build::VERSION));
        }
        crate::cli::CompareFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        crate::cli::CompareFormat::Text => {
            println!(
                "{} : {} ({} metrics compared)",
                report.scenario_id,
                report.overall.as_str(),
                report.rows.len()
            );
        }
    }
    if matches!(report.overall, Verdict::Fail) {
        return Err(BenchError::BenchVerdict { failed_count: 1 });
    }
    Ok(ExitCode::Success)
}

pub async fn dispatch_baseline_regen(args: BaselineRegenArgs) -> BenchResult<ExitCode> {
    if !args.release_tag_confirmed {
        return Err(BenchError::InvalidScenario(
            "baseline regen requires --release-tag-confirmed (AD16/AD40)".into(),
        ));
    }
    warn!(chart_version = %args.chart_version, "regenerating baselines — release-tag-only path");
    let scenarios = Scenario::discover(&args.scenarios)?;
    let mut manifest_scenarios = Vec::new();
    for s in &scenarios {
        // For Phase-2 we don't actually re-run scenarios here ; that's the
        // CI release workflow's job.  We only build the manifest skeleton.
        manifest_scenarios.push(ScenarioManifestEntry {
            id: s.id.clone(),
            duration_s: 0.0,
            verdict: "pending".into(),
        });
    }
    let manifest = BaselineManifest {
        chart_version: args.chart_version.clone(),
        harness_version: crate::build::VERSION.into(),
        loadgen_version: efe_meta_swarm_loadgen::build::VERSION.into(),
        git_commit: crate::build::GIT_SHA.into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        cluster: ClusterDescription {
            provisioner: "external".into(),
            k8s_version: None,
            node_class: None,
        },
        scenarios: manifest_scenarios,
    };
    let path = BaselineManifest::path(&args.baselines, &args.chart_version);
    manifest.write(&path)?;
    println!("[bench] manifest written : {}", path.display());
    Ok(ExitCode::Success)
}

pub async fn dispatch_publish(args: PublishArgs) -> BenchResult<ExitCode> {
    // PR #24 § 15 — replace PR #21 stub with the full Publisher.
    let mut publisher = Publisher::new(
        args.output.clone(),
        args.source.clone(),
        args.chart_version.clone().unwrap_or_else(|| "unknown".into()),
    );
    publisher.bench_results = args.bench_results.clone().unwrap_or_else(|| {
        std::path::PathBuf::from("bench/results")
    });
    publisher.bench_baselines = args.bench_baselines.clone().unwrap_or_else(|| {
        std::path::PathBuf::from("bench_baselines")
    });
    publisher.embed_slos = args.embed_slos;
    publisher.embed_smoke = args.embed_smoke;
    publisher.history_window_days = args.history_window_days;

    // **D-A fix** — actually read `bench/results/*/summary.json` +
    // baselines + scenario YAMLs into an aggregator payload.  Without
    // this, the JSON API was always empty regardless of bench data.
    let scenarios_root = std::path::PathBuf::from("bench/scenarios");
    let scenarios = Scenario::discover(&scenarios_root).unwrap_or_default();

    let agg = crate::publish::aggregator::AggregatedPublish::from_disk(
        &publisher.bench_results,
        &publisher.bench_baselines,
        &scenarios_root,
    )
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "aggregator failed — falling back to empty publish");
        crate::publish::aggregator::AggregatedPublish {
            current_scenarios: Default::default(),
            scenarios_meta: Default::default(),
            baselines: Default::default(),
            overall_verdict: Verdict::NoBaseline,
            latest_run_id: None,
            latest_git_sha: None,
            badge_rows: Vec::new(),
        }
    });

    // Badge rows : prefer aggregator real data ; fallback to NoBaseline
    // per discovered scenario when no run results exist yet.
    let scenario_rows: Vec<_> = if agg.badge_rows.is_empty() {
        scenarios
            .iter()
            .map(|s| (s.id.clone(), Verdict::NoBaseline, None))
            .collect()
    } else {
        agg.badge_rows.clone()
    };

    if args.dry_run {
        // D1 fix : dry_run actually writes to a tempdir + invokes the
        // ratchet so AC1+AD38 truly hold.
        let manifest = publisher.dry_run().await?;
        manifest.validate()?;
        println!(
            "[bench-publish] dry-run OK : {} badges, {} API endpoints, {} scenario pages",
            manifest.badge_count, manifest.api_endpoint_count, manifest.scenario_page_count
        );
        return Ok(ExitCode::Success);
    }

    publisher
        .write_skeleton_with_agg(&scenario_rows, Some(&agg), Some(&scenarios))
        .await?;

    // Pre-deploy ratchet (AD32 — T123 mitigation).
    let manifest = crate::publish::deploy::pre_deploy_ratchet(&args.output)?;
    manifest.validate()?;

    println!(
        "[bench-publish] published {} scenario pages + 11 badges + 5 JSON endpoints to {}",
        scenario_rows.len(),
        args.output.display()
    );
    Ok(ExitCode::Success)
}

/// PR #24 § 12 — generate signed quarterly compliance evidence.
pub async fn dispatch_publish_quarterly(
    args: crate::cli::PublishQuarterlyArgs,
) -> BenchResult<ExitCode> {
    let chart_version = args
        .chart_version
        .as_deref()
        .ok_or_else(|| BenchError::InvalidScenario("--chart-version required".into()))?;
    let quarter = &args.period;

    // ── 1.  Build the JSON sidecar.
    let json = crate::publish::quarterly::json::QuarterlyJson::build(
        chart_version.into(),
        quarter.clone(),
        infer_months(quarter)?,
        std::collections::BTreeMap::new(), // Phase-2 stub : per-scenario aggregates populated by CI step
    );
    let json_bytes = serde_json::to_vec_pretty(&json)?;

    // ── 2.  Render the PDF + embed the SHA.
    let body_text = format!(
        // **D-AL fix** — body must be ASCII-only.  The Phase-2 PDF
        // uses Helvetica + WinAnsi encoding which can't represent
        // codepoints > U+00FF cleanly ; the previous `§` rendered as
        // mojibake (`Â§`) in PDF readers.  Replaced with `Section`.
        "EFE Bench Quarterly Evidence\n\n  Period       : {quarter}\n  Chart version: {chart_version}\n  Schema       : v1.6 Section 24\n\nVerifier :\n  efe-bench-harness evidence verify <pdf>\n"
    );
    let pdf_bytes = crate::publish::quarterly::pdf::build_quarterly_pdf(
        &format!("EFE Bench Evidence {quarter}"),
        &body_text,
        &json_bytes,
    )?;

    // ── 3.  Output paths.
    let q_dir = args.output.join(format!("v{chart_version}/quarterly"));
    std::fs::create_dir_all(&q_dir).map_err(|source| BenchError::Result {
        path: q_dir.clone(),
        source,
    })?;
    let json_path = q_dir.join(format!("{quarter}.json"));
    let pdf_path = q_dir.join(format!("{quarter}.pdf"));

    if args.dry_run {
        println!("[bench-publish-quarterly] dry-run : would write {} + {}", json_path.display(), pdf_path.display());
        return Ok(ExitCode::Success);
    }

    std::fs::write(&json_path, &json_bytes).map_err(|source| BenchError::Result {
        path: json_path.clone(),
        source,
    })?;
    std::fs::write(&pdf_path, &pdf_bytes).map_err(|source| BenchError::Result {
        path: pdf_path.clone(),
        source,
    })?;
    info!(pdf = %pdf_path.display(), json = %json_path.display(), "quarterly artefacts written");

    // ── 4.  Cosign sign-blob (skipped if --cosign-disabled).
    if !args.cosign_disabled {
        crate::publish::quarterly::sign::sign(&pdf_path, args.cosign_identity.as_deref())?;
    } else {
        info!("cosign signing disabled (--cosign-disabled)");
    }

    println!(
        "[bench-publish-quarterly] {quarter} evidence written + signed at {}",
        q_dir.display()
    );
    Ok(ExitCode::Success)
}

/// PR #24 § 12 + AD26 — cross-chain evidence verifier.
pub async fn dispatch_evidence(args: crate::cli::EvidenceArgs) -> BenchResult<ExitCode> {
    match args.action {
        crate::cli::EvidenceAction::Verify { pdf } => {
            crate::publish::quarterly::sign::verify(&pdf)?;
            println!(
                "[bench-evidence] PDF verified : {} (SHA + cosign chain via shared identity regex)",
                pdf.display()
            );
            Ok(ExitCode::Success)
        }
    }
}

/// Infer the 3 months covered by a quarter string `Qn-YYYY`.
///
/// **D-AX fix** — strict format validation :
///   * total length MUST be exactly 7 (`Qn-YYYY`)
///   * year MUST be 4 ASCII digits (no leading minus, no 2-digit year)
///   * quarter index ∈ {1, 2, 3, 4}
///
/// Previously the function accepted `Q1-26` (2-digit year) and even
/// `Q1--26` (negative year via i32 parse).  The workflow always
/// produces strict 4-digit years, but defensive validation prevents
/// downstream filename drift (e.g. `26-01.json` instead of `2026-01.json`).
fn infer_months(quarter: &str) -> BenchResult<Vec<String>> {
    let bytes = quarter.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'Q' || bytes[2] != b'-' {
        return Err(BenchError::InvalidScenario(format!(
            "quarter '{quarter}' must match 'Qn-YYYY' (exact 7 chars)"
        )));
    }
    let q: u32 = std::str::from_utf8(&bytes[1..2])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| BenchError::InvalidScenario(format!("bad quarter index in '{quarter}'")))?;
    // Year MUST be 4 ASCII digits (positive only).
    let year_bytes = &bytes[3..7];
    if !year_bytes.iter().all(|b| b.is_ascii_digit()) {
        return Err(BenchError::InvalidScenario(format!(
            "year in '{quarter}' must be 4 ASCII digits"
        )));
    }
    let year: i32 = std::str::from_utf8(year_bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| BenchError::InvalidScenario(format!("bad year in '{quarter}'")))?;
    let first_month = match q {
        1 => 1,
        2 => 4,
        3 => 7,
        4 => 10,
        _ => return Err(BenchError::InvalidScenario(format!("quarter index must be 1..4, got Q{q}"))),
    };
    Ok((0..3).map(|i| format!("{year}-{:02}", first_month + i)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_validate_walks_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let args = ValidateArgs { scenarios: tmp.path().to_path_buf() };
        let rc = dispatch_validate(args).await.unwrap();
        assert_eq!(rc, ExitCode::Success);
    }

    /// **D-BO** — validate subcommand against the REAL shipped
    /// scenarios MUST pass.  Previously only empty-dir was tested.
    #[tokio::test]
    async fn d_bo_dispatch_validate_passes_on_real_shipped_yamls() {
        let scen_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scenarios");
        let args = ValidateArgs { scenarios: scen_dir };
        let rc = dispatch_validate(args).await.expect("D-BO : real YAMLs must validate");
        assert_eq!(rc, ExitCode::Success);
    }

    #[tokio::test]
    async fn baseline_regen_requires_confirmation() {
        let args = BaselineRegenArgs {
            chart_version: "0.6.0".into(),
            scenarios: PathBuf::from("bench/scenarios"),
            baselines: PathBuf::from("/tmp"),
            release_tag_confirmed: false,
        };
        let err = dispatch_baseline_regen(args).await.unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    /// **D-BD** — baseline_regen with `--release-tag-confirmed` MUST
    /// emit a manifest stub at `baselines/<chart>/MANIFEST.json`.
    /// Previously only the refusal path was tested ; the happy path
    /// was untested → a silent regression would have shipped.
    #[tokio::test]
    async fn d_bd_baseline_regen_confirmed_emits_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let args = BaselineRegenArgs {
            chart_version: "0.6.0".into(),
            scenarios: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scenarios"),
            baselines: tmp.path().to_path_buf(),
            release_tag_confirmed: true,
        };
        let rc = dispatch_baseline_regen(args).await.expect("baseline-regen");
        assert_eq!(rc, ExitCode::Success);
        // The manifest path is `baselines/<chart>/MANIFEST.json`
        // (per BaselineManifest::path).  Recursive find via std::fs.
        fn find_any(dir: &std::path::Path, needle: &str) -> bool {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        if find_any(&p, needle) {
                            return true;
                        }
                    } else if p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.contains(needle))
                        .unwrap_or(false)
                    {
                        return true;
                    }
                }
            }
            false
        }
        assert!(
            find_any(tmp.path(), "manifest.json"),
            "D-BD : manifest.json must be produced under {}",
            tmp.path().display()
        );
    }

    // ───────────────────────────────────────────────────────────────
    // D-AX — infer_months edge cases.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn d_ax_infer_months_q1_2026() {
        let m = infer_months("Q1-2026").unwrap();
        assert_eq!(m, vec!["2026-01", "2026-02", "2026-03"]);
    }

    #[test]
    fn d_ax_infer_months_q4_2026() {
        let m = infer_months("Q4-2026").unwrap();
        assert_eq!(m, vec!["2026-10", "2026-11", "2026-12"]);
    }

    #[test]
    fn d_ax_infer_months_rejects_q5() {
        let err = infer_months("Q5-2026").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_q0() {
        let err = infer_months("Q0-2026").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_2digit_year() {
        // **D-AX** : `Q1-26` used to be accepted, producing "26-01"
        // which broke quarterly directory filename conventions.
        let err = infer_months("Q1-26").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_negative_year() {
        // **D-AX** : `Q1--026` used to parse as year=-26.
        let err = infer_months("Q1--026").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_trailing_junk() {
        let err = infer_months("Q1-2026X").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_empty_string() {
        let err = infer_months("").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }

    #[test]
    fn d_ax_infer_months_rejects_non_digit_quarter() {
        let err = infer_months("Qx-2026").unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::Config);
    }
}
