//! PR #24 § 15 — bench public-page publisher.
//!
//! Replaces the PR #21 stub.  See
//! `docs/EFE_A1_PHASE2_PR24_ARCHITECTURE.md` for the full design.
//!
//! ## Module map
//!
//! | Module | Purpose |
//! |---|---|
//! | [`badge`]     | 11 byte-stable SVG badges (10 scenarios + overall) |
//! | [`trend`]     | 30-day sparkline SVG |
//! | [`api`]       | 5 frozen JSON API endpoints |
//! | [`site`]      | mdbook orchestration + layout verification |
//! | [`crosslink`] | PR #22 SLO + PR #23 smoke embedding |
//! | [`quarterly`] | signed PDF + JSON sidecar + cosign |
//! | [`deploy`]    | gh-pages deploy + ratchet sanity check |
//!
//! ## Stability
//!
//! STABILITY_CONTRACT v1.6 § 24 — frozen for ≥ 6 chart-minor versions.

pub mod aggregator;
pub mod api;
pub mod badge;
pub mod crosslink;
pub mod deploy;
pub mod quarterly;
pub mod site;
pub mod trend;

use std::path::PathBuf;

use crate::error::{BenchError, BenchResult};

/// Top-level publisher orchestrator.
///
/// Idempotent (AD37) : re-running with identical inputs produces
/// byte-identical output bytes.
pub struct Publisher {
    /// Where to write the gh-pages content (the operator typically
    /// passes a tempdir then `peaceiris/actions-gh-pages` ships it).
    pub output_dir: PathBuf,
    /// mdbook source directory (default = `bench/publish/src`).
    pub source_dir: PathBuf,
    /// Directory holding per-run `summary.json` artefacts.
    pub bench_results: PathBuf,
    /// Directory holding `bench_baselines/v*/L2/*.json`.
    pub bench_baselines: PathBuf,
    pub chart_version: String,
    /// AD34 — opt-in PR #22 SLO snapshot embed.
    pub embed_slos: bool,
    /// AD35 — embed PR #23 smoke badge in overview header (default on).
    pub embed_smoke: bool,
    /// AD11 — 30-day rolling window for trend.
    pub history_window_days: u32,
}

impl Publisher {
    /// Construct a new publisher.
    #[must_use]
    pub fn new(output_dir: PathBuf, source_dir: PathBuf, chart_version: String) -> Self {
        Self {
            output_dir,
            source_dir,
            bench_results: PathBuf::from("bench/results"),
            bench_baselines: PathBuf::from("bench_baselines"),
            chart_version,
            embed_slos: false,
            embed_smoke: true,
            history_window_days: 30,
        }
    }

    /// **D1 fix** — actually run the publisher against a tempdir +
    /// invoke the ratchet (AD38).  Returns the inspected manifest +
    /// guarantees the publisher would produce a valid layout.
    pub async fn dry_run(&self) -> BenchResult<deploy::DeployManifest> {
        let tmp = tempfile::tempdir().map_err(|source| BenchError::Result {
            path: std::path::PathBuf::from("(tempdir)"),
            source,
        })?;
        // Build a "throwaway" publisher pointing at the tempdir.
        let staging = Self {
            output_dir: tmp.path().to_path_buf(),
            source_dir: self.source_dir.clone(),
            bench_results: self.bench_results.clone(),
            bench_baselines: self.bench_baselines.clone(),
            chart_version: self.chart_version.clone(),
            embed_slos: false, // never hit network in dry-run
            embed_smoke: self.embed_smoke,
            history_window_days: self.history_window_days,
        };
        // Synthetic scenarios so the badge generator has something
        // to write.  10 NoBaseline rows — matches the PR #21 first-
        // publish state.
        let scenarios = synthetic_scenario_rows();
        staging.write_skeleton(&scenarios).await?;
        deploy::pre_deploy_ratchet(tmp.path())
    }

    /// Generate `output_dir/badges/{11}.svg`.
    pub fn write_badges(&self, scenarios: &[(String, crate::compare::Verdict, Option<f64>)]) -> BenchResult<()> {
        let dir = self.output_dir.join("badges");
        std::fs::create_dir_all(&dir).map_err(|source| BenchError::Result {
            path: dir.clone(),
            source,
        })?;

        // Per-scenario badges.
        for (id, verdict, p95) in scenarios {
            let short = short_id(id);
            let badge = badge::ScenarioBadge::new(short.clone(), *verdict, *p95);
            let filename = format!("scenario-{short}.svg");
            let path = dir.join(&filename);
            std::fs::write(&path, badge.render()).map_err(|source| BenchError::Result {
                path: path.clone(),
                source,
            })?;
        }

        // Overall badge.
        let verdicts: Vec<_> = scenarios.iter().map(|(_, v, _)| *v).collect();
        let overall = badge::OverallBadge::from_scenarios(&verdicts);
        let path = dir.join("overall.svg");
        std::fs::write(&path, overall.render()).map_err(|source| BenchError::Result {
            path: path.clone(),
            source,
        })?;
        Ok(())
    }

    /// Generate `output_dir/api/{current,history,scenarios,baselines,version}.json`.
    ///
    /// Backward-compatible : called by legacy tests + non-aggregator
    /// paths.  Writes empty stubs for scenarios.json + baselines.json.
    /// Use [`Self::write_api_with_agg`] for the real (data-populated)
    /// version.
    pub async fn write_api(&self, current: &api::current::CurrentJson) -> BenchResult<()> {
        self.write_api_with_agg(current, None).await
    }

    /// **D-C fix** — write the JSON API endpoints, populating
    /// `scenarios.json` + `baselines.json` from the aggregator when
    /// available.  Falls back to empty stubs when `agg` is `None`.
    pub async fn write_api_with_agg(
        &self,
        current: &api::current::CurrentJson,
        agg: Option<&aggregator::AggregatedPublish>,
    ) -> BenchResult<()> {
        let dir = self.output_dir.join("api");
        std::fs::create_dir_all(&dir).map_err(|source| BenchError::Result {
            path: dir.clone(),
            source,
        })?;
        api::write_canonical(&dir.join("current.json"), current)?;

        // history.json : empty on first publish ; populated by past-day
        // ingestion in CI workflows (out-of-band — Phase-3 G.10).
        api::write_canonical(
            &dir.join("history.json"),
            &api::history::HistoryJson::new(vec![]),
        )?;

        // scenarios.json : real metadata when agg present.
        let scenarios_meta = agg
            .map(|a| a.scenarios_meta.clone())
            .unwrap_or_default();
        api::write_canonical(
            &dir.join("scenarios.json"),
            &api::scenarios::ScenariosJson::new(scenarios_meta),
        )?;

        // baselines.json : real chart-version map when agg present.
        let baselines_map = agg
            .map(|a| a.baselines.clone())
            .unwrap_or_default();
        api::write_canonical(
            &dir.join("baselines.json"),
            &api::baselines::BaselinesJson::new(baselines_map),
        )?;

        api::write_canonical(
            &dir.join("version.json"),
            &api::version::VersionJson::build(self.chart_version.clone()),
        )?;
        Ok(())
    }

    /// **D-D + D-K fix** — generate a 30-day sparkline SVG per scenario
    /// at `/badges/trend-{short}.svg`.
    ///
    /// Phase-2 : no historical time-series is available (history.json
    /// is populated by an out-of-band CI ingestion step, Phase-3 G.10).
    /// We therefore emit a single-point sparkline for "we have today's
    /// data only" OR an empty "no data" sparkline otherwise.
    ///
    /// **D-K** : the verdict colour is taken from the aggregator's
    /// per-scenario badge_rows when available (was hardcoded to
    /// NoBaseline previously — sparklines silently disagreed with the
    /// scenario badge).
    pub async fn write_sparklines(
        &self,
        scenarios: &[crate::scenario::Scenario],
        agg: Option<&aggregator::AggregatedPublish>,
    ) -> BenchResult<()> {
        let dir = self.output_dir.join("badges");
        std::fs::create_dir_all(&dir).map_err(|source| BenchError::Result {
            path: dir.clone(),
            source,
        })?;
        // Build a fast (scenario_id → (verdict, p95)) lookup from the agg.
        let by_id: std::collections::HashMap<&str, (crate::compare::Verdict, Option<f64>)> = agg
            .map(|a| {
                a.badge_rows
                    .iter()
                    .map(|(id, v, p)| (id.as_str(), (*v, *p)))
                    .collect()
            })
            .unwrap_or_default();
        for s in scenarios {
            let short = short_id(&s.id);
            let (verdict, p95) = by_id
                .get(s.id.as_str())
                .copied()
                .unwrap_or((crate::compare::Verdict::NoBaseline, None));
            // Single-point series : today's p95 (if any).  When no p95
            // is available the points slice stays empty → "no data".
            let points: Vec<trend::DailyPoint> = match p95 {
                Some(v) => vec![trend::DailyPoint { day_index: 29, p95_ms: Some(v) }],
                None => Vec::new(),
            };
            let svg = trend::render(&points, verdict);
            let path = dir.join(format!("trend-{short}.svg"));
            std::fs::write(&path, svg).map_err(|source| BenchError::Result {
                path: path.clone(),
                source,
            })?;
        }
        Ok(())
    }

    /// Convenience : write all minimum required artefacts so the
    /// ratchet passes.  Used by tests + `--dry-run`.
    ///
    /// **D2 fix** : if mdbook is available, run it FIRST so the real
    /// HTML content from `bench/publish/src/*.md` is produced.  Then
    /// fill in any missing required page with a CSP-safe placeholder.
    ///
    /// **D-B fix** : the passed `scenarios` tuples are now propagated
    /// into `/api/current.json` (previously the JSON was always empty
    /// while the badges carried real data — inconsistent surface).
    pub async fn write_skeleton(
        &self,
        scenarios: &[(String, crate::compare::Verdict, Option<f64>)],
    ) -> BenchResult<()> {
        self.write_skeleton_with_agg(scenarios, None, None).await
    }

    /// PR #24 § 15 — full pipeline accepting an optional aggregator
    /// payload.  When `agg` is `Some`, the JSON API + sparklines are
    /// populated from real bench data ; when `None`, only the
    /// `scenarios` badge rows are honoured (Phase-2 minimum).
    pub async fn write_skeleton_with_agg(
        &self,
        scenarios: &[(String, crate::compare::Verdict, Option<f64>)],
        agg: Option<&aggregator::AggregatedPublish>,
        scenario_objs: Option<&[crate::scenario::Scenario]>,
    ) -> BenchResult<()> {
        // 1.  Badges (always — independent of mdbook).
        self.write_badges(scenarios)?;

        // 2.  API : propagate aggregator data when available (D-B fix).
        let (current_scenarios, overall_verdict, run_id, git_sha) = match agg {
            Some(a) => (
                a.current_scenarios.clone(),
                a.overall_verdict,
                a.latest_run_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
                a.latest_git_sha
                    .clone()
                    .unwrap_or_else(|| crate::build::GIT_SHA.to_string()),
            ),
            None => {
                // Fall back : synthesise from `scenarios` tuples.
                let mut map = std::collections::BTreeMap::new();
                for (id, verdict, p95) in scenarios {
                    map.insert(
                        id.clone(),
                        api::current::CurrentScenarioEntry {
                            verdict: verdict.as_str().into(),
                            p50_ms: None,
                            p95_ms: *p95,
                            p99_ms: None,
                            error_rate: None,
                            throughput_qps: None,
                            tolerance_used: std::collections::BTreeMap::new(),
                        },
                    );
                }
                let worst = scenarios
                    .iter()
                    .map(|(_, v, _)| *v)
                    .fold(crate::compare::Verdict::Pass, |a, b| a.combine(b));
                (
                    map,
                    worst,
                    uuid::Uuid::now_v7().to_string(),
                    crate::build::GIT_SHA.to_string(),
                )
            }
        };
        let current = api::current::CurrentJson::build(
            self.chart_version.clone(),
            run_id,
            git_sha,
            overall_verdict,
            current_scenarios,
            if self.embed_slos {
                crosslink::slo::fetch_slo_snapshot().await.unwrap_or(None)
            } else {
                None
            },
        );
        self.write_api_with_agg(&current, agg).await?;

        // 2b.  Trend sparklines (D-D fix) — generate /badges/trend-SN.svg
        //      per scenario.  Best-effort — uses Phase-2 stub data when
        //      no history available.
        if let Some(scens) = scenario_objs {
            self.write_sparklines(scens, agg).await?;
        }

        // 3.  **D2 fix** — try mdbook first (best-effort).  When it
        //     succeeds, the real per-scenario HTML is generated.
        if self.source_dir.exists() {
            let _ = site::build_mdbook(&self.source_dir, &self.output_dir);
            // **D-N fix** — substitute the smoke-badge sentinel in the
            // mdbook-produced index.html so `--embed-smoke` actually
            // gates the badge in BOTH code paths (mdbook + placeholder).
            // Without this, the flag was a no-op when mdbook was on PATH.
            let _ = site::post_process_smoke_slot(&self.output_dir, self.embed_smoke);
        }

        // **D-J** — build the smoke badge fragment ONCE.  Inject it into
        //     index.html ONLY (matches AD35 : overview header).  When
        //     `--embed-smoke false` is passed, the fragment is empty
        //     → flag now has a visible effect.
        let smoke_fragment = if self.embed_smoke {
            Some(crosslink::smoke::render_header_html())
        } else {
            None
        };

        // 4.  Required top-level pages : only write CSP-safe placeholders
        //     for files mdbook didn't produce.
        for f in ["index.html", "methodology.html", "chaos.html", "quarterly-archive.html"] {
            let path = self.output_dir.join(f);
            if !path.exists() {
                let extras = if f == "index.html" {
                    smoke_fragment.as_deref()
                } else {
                    None
                };
                std::fs::write(&path, csp_placeholder_html(f, extras))
                    .map_err(|source| BenchError::Result {
                        path: path.clone(),
                        source,
                    })?;
            }
        }
        // 5.  10 scenario pages — same placeholder-only-if-missing pattern.
        let scen_dir = self.output_dir.join("scenarios");
        std::fs::create_dir_all(&scen_dir).map_err(|source| BenchError::Result {
            path: scen_dir.clone(),
            source,
        })?;
        for (_, f) in SCENARIO_PAGE_FILENAMES {
            let path = scen_dir.join(f);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, csp_placeholder_html(f, None))
                .map_err(|source| BenchError::Result {
                    path: path.clone(),
                    source,
                })?;
        }
        Ok(())
    }
}

/// **D-U fix** — canonical (scenario_id → mdbook_html_filename) mapping
/// — the single source of truth shared between :
///   * the placeholder writer in `write_skeleton_with_agg`
///   * the aggregator's `scenarios.json::page` URL builder
///   * the deploy ratchet's required-page list
///   * the `verify_layout` checker
///
/// Adding a new scenario REQUIRES updating this table.  Without it,
/// the aggregator emits a `page` URL whose target file does not exist
/// — silent 404s in production (D-U historical defect).
pub const SCENARIO_PAGE_FILENAMES: &[(&str, &str)] = &[
    ("S01_baseline_100qps_5min",    "s01-baseline.html"),
    ("S02_canary_1000qps_15min",    "s02-canary.html"),
    ("S03_ramp_to_breakdown",       "s03-ramp.html"),
    ("S04_chaos_participant_loss",  "s04-chaos-participant-loss.html"),
    ("S05_chaos_network_partition", "s05-chaos-network-partition.html"),
    ("S06_chaos_cert_expiry",       "s06-chaos-cert-expiry.html"),
    ("S07_chaos_reload_storm",      "s07-chaos-reload-storm.html"),
    ("S08_multi_region_replay",     "s08-multi-region.html"),
    ("S09_soak_2h_stability",       "s09-soak.html"),
    ("S10_slo_validation_30min",    "s10-slo-validation.html"),
];

#[cfg(test)]
mod page_filenames_invariants {
    use super::SCENARIO_PAGE_FILENAMES;
    use std::collections::HashSet;

    /// **D-AF** — every scenario id MUST be unique.  A duplicate entry
    /// would make the aggregator emit two pages for the same scenario
    /// (and one would be silently overwritten on disk).
    #[test]
    fn d_af_scenario_ids_unique() {
        let ids: HashSet<_> = SCENARIO_PAGE_FILENAMES.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids.len(),
            SCENARIO_PAGE_FILENAMES.len(),
            "D-AF : duplicate scenario_id in SCENARIO_PAGE_FILENAMES"
        );
    }

    /// **D-AF** — every output filename MUST be unique.  A duplicate
    /// filename would mean two scenarios resolve to the same html page.
    #[test]
    fn d_af_filenames_unique() {
        let names: HashSet<_> = SCENARIO_PAGE_FILENAMES.iter().map(|(_, n)| *n).collect();
        assert_eq!(
            names.len(),
            SCENARIO_PAGE_FILENAMES.len(),
            "D-AF : duplicate filename in SCENARIO_PAGE_FILENAMES"
        );
    }

    /// **D-AF** — count locked at 10 (STABILITY § 24.5).  Adding an
    /// 11th scenario without bumping STABILITY is a contract break.
    #[test]
    fn d_af_count_is_10_v1_6() {
        assert_eq!(
            SCENARIO_PAGE_FILENAMES.len(),
            10,
            "D-AF : SCENARIO_PAGE_FILENAMES count diverged from STABILITY v1.6 § 24.5"
        );
    }

    /// **D-AF** — every filename MUST be valid for the gh-pages URL
    /// invariants (§ 24.6 : kebab-case lowercase, .html extension).
    #[test]
    fn d_af_filenames_obey_section_24_6_invariants() {
        for (_, name) in SCENARIO_PAGE_FILENAMES {
            assert!(
                name.ends_with(".html"),
                "D-AF : '{name}' must end with .html (§ 24.6)"
            );
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'),
                "D-AF : '{name}' must be kebab-case lowercase (§ 24.6)"
            );
            assert!(
                !name.contains('_'),
                "D-AF : '{name}' must not contain '_' (kebab-case § 24.6)"
            );
        }
    }

    /// **D-AF** — every scenario_id MUST match the actual YAML in
    /// `bench/scenarios/*.yaml`.  Detects rename drift.
    #[test]
    fn d_af_ids_match_yaml_basenames() {
        let scen_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scenarios");
        for (id, _) in SCENARIO_PAGE_FILENAMES {
            let yaml = scen_dir.join(format!("{id}.yaml"));
            assert!(
                yaml.exists(),
                "D-AF : scenario YAML for '{id}' missing at {} — rename drift between table + on-disk scenarios",
                yaml.display()
            );
        }
    }
}

/// Compatibility alias for PR #21 callers : the legacy `Badge` type
/// is still re-exported.  Phase-2 behaviour kept for callers that
/// haven't migrated yet.
#[derive(Debug, Clone)]
pub struct Badge {
    pub scenario_id: String,
    pub verdict: crate::compare::Verdict,
    pub value_label: String,
}

impl Badge {
    /// Build a legacy badge — wraps [`badge::ScenarioBadge`].
    #[must_use]
    pub fn from_verdict(
        scenario_id: String,
        verdict: crate::compare::Verdict,
        p95_ms: Option<f64>,
    ) -> Self {
        let value_label = match (verdict, p95_ms) {
            (crate::compare::Verdict::Pass | crate::compare::Verdict::Warn, Some(v)) => {
                format!("{v:.0} ms")
            }
            (crate::compare::Verdict::Fail, _) => "FAIL".into(),
            (crate::compare::Verdict::NoBaseline, _) => "no baseline".into(),
            (_, None) => verdict.as_str().into(),
        };
        Self {
            scenario_id,
            verdict,
            value_label,
        }
    }

    /// Render via the new byte-stable generator.
    #[must_use]
    pub fn render(&self) -> String {
        let short = short_id(&self.scenario_id);
        // Extract numeric ms from value_label when possible.
        let p95 = self.value_label.trim_end_matches(" ms").parse::<f64>().ok();
        badge::ScenarioBadge::new(short, self.verdict, p95).render()
    }
}

/// Extract the short id (`S01`, `S04`) from the full scenario id
/// (`S01_baseline_100qps_5min`).
#[must_use]
pub fn short_id(scenario_id: &str) -> String {
    scenario_id
        .split('_')
        .next()
        .unwrap_or(scenario_id)
        .to_string()
}

/// **D3 fix** — Content-Security-Policy meta tag for every placeholder
/// HTML file.  T125 mitigation (phishing via JS injection).  AC6.
const CSP_META: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' https://efe.github.io; object-src 'none'; base-uri 'self'; form-action 'none'">"#;

/// Render a CSP-safe placeholder HTML for `page_name`.  Used when
/// mdbook didn't produce the page (e.g. mdbook not installed locally).
///
/// **D-J fix** — when `extras` carries the PR #23 smoke badge fragment
/// (and `page_name == "index.html"`), it is injected into the header.
/// This is what makes `--embed-smoke` actually have a visible effect.
///
/// **D-Y fix** — `<link rel="canonical">` now points to the page's
/// OWN URL instead of always pointing to `/bench/`.  Previously every
/// placeholder claimed to be a duplicate of the root → search engines
/// would de-index every scenario page.
fn csp_placeholder_html(page_name: &str, extras: Option<&str>) -> String {
    let safe_name = page_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .collect::<String>();
    let header_fragment = extras.unwrap_or("");
    let canonical_url = canonical_url_for(&safe_name);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  {CSP_META}
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>EFE Bench — {safe_name}</title>
  <link rel="canonical" href="{canonical_url}">
</head>
<body>
  <header>{header_fragment}</header>
  <p>EFE Public Bench placeholder for <code>{safe_name}</code> — install <code>mdbook</code> and re-run <code>efe-bench-harness publish</code> to render the real content from <code>bench/publish/src/</code>.</p>
  <p>Stability : v1.6 § 24.</p>
</body>
</html>
"#,
    )
}

/// **D-Y helper** — compute the per-page canonical URL for SEO.
///
/// Rules :
///   * `index.html`             → `https://efe.github.io/bench/`
///   * `<top-level>.html`       → `https://efe.github.io/bench/<top-level>.html`
///   * `s0[0-9]-...html` /
///     `s10-...html`            → `https://efe.github.io/bench/scenarios/<file>`
///
/// Returns the kebab-case URL pre-validated for `https-only`, no query
/// strings, explicit file extension (STABILITY § 24.6 invariants).
fn canonical_url_for(safe_name: &str) -> String {
    const BASE: &str = "https://efe.github.io/bench/";
    if safe_name == "index.html" {
        return BASE.to_string();
    }
    // Scenario pages live under /scenarios/.
    if safe_name.starts_with('s') && safe_name.len() >= 3
        && safe_name.as_bytes()[1].is_ascii_digit()
        && safe_name.as_bytes()[2].is_ascii_digit()
    {
        return format!("{BASE}scenarios/{safe_name}");
    }
    // Top-level pages (methodology, chaos, quarterly-archive, ...).
    format!("{BASE}{safe_name}")
}

/// **D1 fix helper** — synthesised 10 scenario rows for the dry-run
/// path.  Phase-2 first-publish state : every scenario is NoBaseline.
fn synthetic_scenario_rows() -> Vec<(String, crate::compare::Verdict, Option<f64>)> {
    [
        "S01_baseline_100qps_5min",
        "S02_canary_1000qps_15min",
        "S03_ramp_to_breakdown",
        "S04_chaos_participant_loss",
        "S05_chaos_network_partition",
        "S06_chaos_cert_expiry",
        "S07_chaos_reload_storm",
        "S08_multi_region_replay",
        "S09_soak_2h_stability",
        "S10_slo_validation_30min",
    ]
    .iter()
    .map(|id| ((*id).to_string(), crate::compare::Verdict::NoBaseline, None))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_extracts_first_segment() {
        assert_eq!(short_id("S01_baseline_100qps_5min"), "S01");
        assert_eq!(short_id("S10_slo_validation_30min"), "S10");
        assert_eq!(short_id("noseg"), "noseg");
    }

    #[test]
    fn d_y_canonical_url_root_for_index() {
        assert_eq!(canonical_url_for("index.html"), "https://efe.github.io/bench/");
    }

    #[test]
    fn d_y_canonical_url_top_level_for_non_scenario_pages() {
        assert_eq!(
            canonical_url_for("methodology.html"),
            "https://efe.github.io/bench/methodology.html"
        );
        assert_eq!(
            canonical_url_for("chaos.html"),
            "https://efe.github.io/bench/chaos.html"
        );
        assert_eq!(
            canonical_url_for("quarterly-archive.html"),
            "https://efe.github.io/bench/quarterly-archive.html"
        );
    }

    #[test]
    fn d_y_canonical_url_scenarios_subdir_for_s_prefixed() {
        assert_eq!(
            canonical_url_for("s01-baseline.html"),
            "https://efe.github.io/bench/scenarios/s01-baseline.html"
        );
        assert_eq!(
            canonical_url_for("s04-chaos-participant-loss.html"),
            "https://efe.github.io/bench/scenarios/s04-chaos-participant-loss.html"
        );
        assert_eq!(
            canonical_url_for("s10-slo-validation.html"),
            "https://efe.github.io/bench/scenarios/s10-slo-validation.html"
        );
    }

    #[test]
    fn d_y_canonical_url_obeys_stability_24_6_invariants() {
        for name in [
            "index.html",
            "methodology.html",
            "s01-baseline.html",
            "s10-slo-validation.html",
        ] {
            let url = canonical_url_for(name);
            assert!(url.starts_with("https://"), "must be https-only");
            assert!(!url.contains('?'), "no query strings (STABILITY § 24.6)");
            assert!(
                url.ends_with('/') || url.ends_with(".html"),
                "explicit extension required (§ 24.6)"
            );
        }
    }

    #[test]
    fn legacy_badge_still_works() {
        let b = Badge::from_verdict("S01_baseline".into(), crate::compare::Verdict::Pass, Some(8.0));
        let svg = b.render();
        assert!(svg.contains("S01"));
        assert!(!svg.contains("FAIL"));
    }

    #[test]
    fn legacy_badge_fail_label() {
        let b = Badge::from_verdict("S04_chaos".into(), crate::compare::Verdict::Fail, None);
        let svg = b.render();
        assert!(svg.contains("FAIL"));
        assert!(svg.contains("#da3633"));
    }

    #[tokio::test]
    async fn write_skeleton_produces_ratchet_passing_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let pub_ = Publisher::new(
            tmp.path().to_path_buf(),
            std::path::PathBuf::from("/tmp/nonexistent-source"),
            "0.6.0".into(),
        );
        let scenarios = vec![
            ("S01_baseline".into(), crate::compare::Verdict::Pass, Some(8.0)),
            ("S02_canary".into(), crate::compare::Verdict::Pass, Some(12.0)),
            ("S03_ramp".into(), crate::compare::Verdict::Pass, Some(45.0)),
            ("S04_chaos_participant_loss".into(), crate::compare::Verdict::Pass, Some(60.0)),
            ("S05_chaos_network_partition".into(), crate::compare::Verdict::Pass, Some(75.0)),
            ("S06_chaos_cert_expiry".into(), crate::compare::Verdict::Pass, Some(40.0)),
            ("S07_chaos_reload_storm".into(), crate::compare::Verdict::Pass, Some(55.0)),
            ("S08_multi_region".into(), crate::compare::Verdict::Pass, Some(90.0)),
            ("S09_soak".into(), crate::compare::Verdict::Pass, Some(50.0)),
            ("S10_slo_validation".into(), crate::compare::Verdict::Pass, None),
        ];
        pub_.write_skeleton(&scenarios).await.unwrap();
        let manifest = crate::publish::deploy::pre_deploy_ratchet(tmp.path()).unwrap();
        manifest.validate().unwrap();
    }

    #[test]
    fn publisher_constructor_defaults() {
        let p = Publisher::new(
            std::path::PathBuf::from("/tmp/o"),
            std::path::PathBuf::from("/tmp/s"),
            "0.6.0".into(),
        );
        assert_eq!(p.history_window_days, 30);
        assert!(!p.embed_slos);
        assert!(p.embed_smoke);
    }
}
