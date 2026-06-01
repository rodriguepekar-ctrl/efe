//! PR #24 § 15 — bench results aggregator.
//!
//! **The core of `efe-bench-harness publish`** : reads
//!   * `bench/results/{run_id}/{scenario_id}/summary.json` files
//!     (PR #21 schema v1 — the canonical per-run output)
//!   * `bench_baselines/v{chart}/L2/{scenario_id}.json` files
//!   * `bench/scenarios/{scenario_id}.yaml` files (PR #21 — metadata)
//!
//! and produces the in-memory data structures consumed by the JSON API
//! serialisers + the badge generators.
//!
//! When `bench/results/` is empty (first-publish state) the aggregator
//! returns a partial dataset — every scenario known via its YAML gets a
//! `NoBaseline` verdict ; `/api/history.json` is empty ; the badges
//! show "no data".

use std::collections::BTreeMap;
use std::path::Path;

use crate::baseline::Baseline;
use crate::compare::Verdict;
use crate::error::{BenchError, BenchResult};
use crate::publish::api::baselines::{BaselineEntry, BaselineSet};
use crate::publish::api::current::CurrentScenarioEntry;
use crate::publish::api::scenarios::ScenarioMeta;
use crate::result::ScenarioResult;
use crate::scenario::Scenario;

/// Aggregated bench-publish payload — ready to feed into the
/// JSON API serialisers + the badge/trend generators.
#[derive(Debug, Clone)]
pub struct AggregatedPublish {
    /// Latest run's per-scenario entries (canonical sorted by ID).
    pub current_scenarios: BTreeMap<String, CurrentScenarioEntry>,
    /// Per-scenario metadata for `/api/scenarios.json`.
    pub scenarios_meta: BTreeMap<String, ScenarioMeta>,
    /// Per-chart-version baseline sets for `/api/baselines.json`.
    pub baselines: BTreeMap<String, BaselineSet>,
    /// Latest run's overall verdict (worst-of).
    pub overall_verdict: Verdict,
    /// Latest `run_id` if any result was found.
    pub latest_run_id: Option<String>,
    /// Latest `git_sha` if any result was found.
    pub latest_git_sha: Option<String>,
    /// (scenario_short_id → (verdict, p95_ms)) for badge generation.
    /// Empty when no results were found ; the badge generator falls
    /// back to NoBaseline.
    pub badge_rows: Vec<(String, Verdict, Option<f64>)>,
}

impl AggregatedPublish {
    /// Walk `bench_results_dir`, `bench_baselines_dir`, `scenarios_dir`
    /// and return the aggregated payload.
    pub fn from_disk(
        bench_results_dir: &Path,
        bench_baselines_dir: &Path,
        scenarios_dir: &Path,
    ) -> BenchResult<Self> {
        // ── 1.  Discover scenarios from YAML.
        let scenarios = Scenario::discover(scenarios_dir).unwrap_or_default();

        // ── 2.  Build scenarios_meta (always, even without results).
        // **D-P fix** — `latest_verdict` MUST use the canonical kebab-case
        // taxonomy string emitted by `Verdict::as_str()` (was `"no_data"`,
        // diverged from the rest of the surface : current.json + badges
        // all use `"no-baseline"`).
        let mut scenarios_meta = BTreeMap::new();
        for s in &scenarios {
            scenarios_meta.insert(
                s.id.clone(),
                ScenarioMeta {
                    description: s.description.clone(),
                    duration: s.duration.clone(),
                    tier: detect_tier(&s.id),
                    chaos_count: s.chaos.len() as u32,
                    latest_verdict: Verdict::NoBaseline.as_str().into(),
                    page: format!(
                        "/scenarios/{}.html",
                        scenario_page_slug(&s.id),
                    ),
                    badge: format!(
                        "/badges/scenario-{}.svg",
                        super::short_id(&s.id),
                    ),
                },
            );
        }

        // ── 3.  Find the most-recent run (by mtime of the run-id dir).
        let latest_run = find_latest_run(bench_results_dir).unwrap_or_default();

        // ── 4.  Per-scenario CurrentJson entries + badge rows.
        let mut current_scenarios = BTreeMap::new();
        let mut badge_rows = Vec::new();
        let mut latest_run_id = None;
        let mut latest_git_sha = None;
        let mut worst = Verdict::NoBaseline;

        for s in &scenarios {
            let summary_path = if !latest_run.is_empty() {
                bench_results_dir
                    .join(&latest_run)
                    .join(&s.id)
                    .join("summary.json")
            } else {
                continue_with_empty_entry(s, &mut current_scenarios, &mut badge_rows);
                continue;
            };
            match ScenarioResult::read(&summary_path) {
                Ok(result) => {
                    latest_run_id.get_or_insert(result.run_id.clone());
                    if latest_git_sha.is_none() {
                        latest_git_sha = Some(result.git_commit.clone());
                    }
                    let verdict = result.verdict.unwrap_or(Verdict::NoBaseline);
                    worst = worst.combine(verdict);
                    let p95 = result.metrics.p95_ms;
                    let entry = CurrentScenarioEntry {
                        verdict: verdict.as_str().into(),
                        p50_ms: result.metrics.p50_ms,
                        p95_ms: p95,
                        p99_ms: result.metrics.p99_ms,
                        error_rate: result.metrics.error_rate,
                        throughput_qps: result.metrics.throughput_qps,
                        // **D-AZ KNOWN GAP** — `tolerance_used` is the
                        // per-metric tolerance threshold that produced
                        // this verdict.  The aggregator does NOT
                        // currently re-run the `compare` pipeline
                        // against the baseline, so this field stays
                        // empty in `/api/current.json`.  Populating it
                        // properly requires :
                        //   1. load `bench_baselines/v{chart}/L2/{id}.json`
                        //   2. run `compare::compare_one()` against the
                        //      summary
                        //   3. flatten the ComparisonReport.rows into
                        //      this map.
                        // Phase-3 G.11 candidate — flagged for future
                        // PR.  Test in tests/aggregator_real_disk.rs
                        // pins the current empty-map behaviour so the
                        // gap surfaces in a regression.
                        tolerance_used: BTreeMap::new(),
                    };
                    current_scenarios.insert(s.id.clone(), entry);
                    if let Some(m) = scenarios_meta.get_mut(&s.id) {
                        m.latest_verdict = verdict.as_str().into();
                    }
                    badge_rows.push((s.id.clone(), verdict, p95));
                }
                Err(e) => {
                    // **D-O fix** — surface the read failure so operators
                    // can distinguish "no run yet" from "schema_version
                    // mismatch" or "JSON corruption".  Previously silent
                    // — appeared identical to a first-publish state.
                    tracing::warn!(
                        scenario = %s.id,
                        path = %summary_path.display(),
                        error = %e,
                        "summary.json unreadable — falling back to NoBaseline"
                    );
                    continue_with_empty_entry(s, &mut current_scenarios, &mut badge_rows);
                }
            }
        }

        // ── 5.  Baselines per chart-version.
        let baselines = load_baselines(bench_baselines_dir).unwrap_or_default();

        Ok(Self {
            current_scenarios,
            scenarios_meta,
            baselines,
            overall_verdict: worst,
            latest_run_id,
            latest_git_sha,
            badge_rows,
        })
    }
}

/// Detect the scenario tier from its ID (heuristic — falls back to "quick").
fn detect_tier(scenario_id: &str) -> String {
    if scenario_id.contains("chaos") {
        "chaos".into()
    } else if scenario_id.contains("soak") {
        "soak".into()
    } else if scenario_id.contains("multi_region") {
        "geo".into()
    } else if scenario_id.contains("slo_validation") {
        "compliance".into()
    } else if scenario_id.contains("canary") || scenario_id.contains("ramp") {
        "perf".into()
    } else {
        "quick".into()
    }
}

/// Convert `S01_baseline_100qps_5min` → `s01-baseline` (page slug).
///
/// **D-U fix** — the previous heuristic (`{parts[0]}-{parts[1]}`)
/// produced wrong slugs for 6 of 10 scenarios :
///   `S04_chaos_participant_loss` → `s04-chaos` ❌  (real : `s04-chaos-participant-loss`)
///   `S08_multi_region_replay`    → `s08-multi` ❌  (real : `s08-multi-region`)
///   `S10_slo_validation_30min`   → `s10-slo`   ❌  (real : `s10-slo-validation`)
///
/// Every `scenarios.json::page` URL pointed to a non-existent file →
/// 404s on user click-through.  The fix is an explicit lookup table
/// that mirrors [`super::SCENARIO_PAGE_FILENAMES`] (single source of
/// truth) ; unknown scenarios fall back to the heuristic for forward-
/// compat with PR #25+ scenarios.
fn scenario_page_slug(scenario_id: &str) -> String {
    if let Some((_, slug)) = super::SCENARIO_PAGE_FILENAMES
        .iter()
        .find(|(sid, _)| *sid == scenario_id)
    {
        // strip the trailing ".html" — page slug is the bare stem.
        return slug.trim_end_matches(".html").to_string();
    }
    // Forward-compat heuristic for unknown scenarios.
    let parts: Vec<&str> = scenario_id.split('_').collect();
    if parts.len() < 2 {
        return scenario_id.to_lowercase().replace('_', "-");
    }
    format!("{}-{}", parts[0].to_lowercase(), parts[1].to_lowercase())
}

fn continue_with_empty_entry(
    s: &Scenario,
    current_scenarios: &mut BTreeMap<String, CurrentScenarioEntry>,
    badge_rows: &mut Vec<(String, Verdict, Option<f64>)>,
) {
    // **D-P fix** — canonical kebab-case `"no-baseline"` (was the
    // diverged `"no_baseline"` snake-case, which broke any frontend
    // matching against `Verdict::as_str()` output).
    current_scenarios.insert(
        s.id.clone(),
        CurrentScenarioEntry {
            verdict: Verdict::NoBaseline.as_str().into(),
            p50_ms: None,
            p95_ms: None,
            p99_ms: None,
            error_rate: None,
            throughput_qps: None,
            tolerance_used: BTreeMap::new(),
        },
    );
    badge_rows.push((s.id.clone(), Verdict::NoBaseline, None));
}

/// Find the most-recent `run_id` directory.
///
/// **D-V fix** — sort by filesystem mtime instead of lexicographic
/// order.  The previous lex-sort assumed every run_id used UUID v7
/// (timestamp-ordered hex prefix) ; any other naming scheme (e.g. a
/// human-friendly `run-pr24-test` or a non-UUID CI identifier) sorted
/// arbitrarily.  Falls back to lex order on mtime read failure.
///
/// **D-AJ defense-in-depth** — reject entries whose name contains
/// path-traversal metacharacters or starts with `.` (hidden / dotfile
/// directories).  In practice `read_dir` never returns `.` or `..`,
/// and the OS forbids creating a basename like `..`, BUT we are not
/// the only producers of files under `bench/results/` (CI runners,
/// operators, mounted volumes).  A defensive filter keeps the
/// aggregator honest even when a hostile process lands a malformed
/// directory name.
fn find_latest_run(bench_results_dir: &Path) -> BenchResult<String> {
    if !bench_results_dir.exists() {
        return Ok(String::new());
    }
    let mut entries: Vec<(String, std::time::SystemTime)> = std::fs::read_dir(bench_results_dir)
        .map_err(|source| BenchError::Result {
            path: bench_results_dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            // **D-AJ** — drop suspicious basenames.
            let name = e.file_name();
            let s = name.to_string_lossy();
            if s.is_empty() || s == "." || s == ".." {
                return false;
            }
            if s.starts_with('.') {
                // Hidden dirs are never legitimate run_id outputs ;
                // operators using `.cache`, `.tmp`, etc. would
                // otherwise be picked up as fake runs.
                return false;
            }
            // Any path metacharacter is rejected.
            !s.contains('/') && !s.contains('\\') && !s.contains('\0')
        })
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // Fall back to UNIX_EPOCH if mtime is unreadable — that way
            // the entry still participates in the sort (lex-tie-break
            // happens below for entries sharing the same mtime).
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (name, mtime)
        })
        .collect();
    // Primary key : mtime ASC ; secondary : name ASC (tie-break for
    // entries written in the same fs-mtime tick, common on CI).
    entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(entries.pop().map(|(n, _)| n).unwrap_or_default())
}

/// Load all `bench_baselines/v*/L2/*.json` files into a per-chart-version map.
fn load_baselines(bench_baselines_dir: &Path) -> BenchResult<BTreeMap<String, BaselineSet>> {
    let mut out = BTreeMap::new();
    if !bench_baselines_dir.exists() {
        return Ok(out);
    }
    let pattern = bench_baselines_dir.join("v*/L2/*.json");
    let entries: Vec<_> = glob::glob(&pattern.to_string_lossy())
        .map_err(|e| BenchError::InvalidScenario(format!("glob : {e}")))?
        .filter_map(Result::ok)
        .collect();
    for path in entries {
        // Path looks like `.../bench_baselines/v0.6.0/L2/S01_xxx.json`.
        let chart_version = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix('v'))
            .unwrap_or("unknown")
            .to_string();
        let scenario_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        // Read for the `started_at` field → "released_at".
        let released_at = Baseline::load(&path)
            .map(|b| b.started_at)
            .unwrap_or_else(|_| String::new());
        let set = out
            .entry(chart_version.clone())
            .or_insert_with(|| BaselineSet {
                released_at: released_at.clone(),
                scenarios: vec![],
            });
        // Replace `released_at` if empty (first iteration won the race).
        if set.released_at.is_empty() && !released_at.is_empty() {
            set.released_at = released_at;
        }
        let url = format!("/baselines/v{chart_version}/L2/{scenario_id}.json");
        set.scenarios.push(BaselineEntry {
            id: scenario_id,
            url,
        });
    }
    // Sort each scenarios vec for canonical output.
    for set in out.values_mut() {
        set.scenarios.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tier_chaos() {
        assert_eq!(detect_tier("S04_chaos_participant_loss"), "chaos");
        assert_eq!(detect_tier("S07_chaos_reload_storm"), "chaos");
    }

    #[test]
    fn detect_tier_soak() {
        assert_eq!(detect_tier("S09_soak_2h_stability"), "soak");
    }

    #[test]
    fn detect_tier_geo() {
        assert_eq!(detect_tier("S08_multi_region_replay"), "geo");
    }

    #[test]
    fn detect_tier_compliance() {
        assert_eq!(detect_tier("S10_slo_validation_30min"), "compliance");
    }

    #[test]
    fn detect_tier_perf() {
        assert_eq!(detect_tier("S02_canary_1000qps_15min"), "perf");
        assert_eq!(detect_tier("S03_ramp_to_breakdown"), "perf");
    }

    #[test]
    fn detect_tier_quick_default() {
        assert_eq!(detect_tier("S01_baseline_100qps_5min"), "quick");
    }

    #[test]
    fn d_u_page_slug_matches_real_mdbook_filenames() {
        // **D-U** — every slug emitted by the aggregator MUST equal the
        // bare stem of a real `bench/publish/src/scenarios/*.md` file.
        // Previous heuristic broke 6 of 10 scenarios → 404 click-through.
        assert_eq!(scenario_page_slug("S01_baseline_100qps_5min"),    "s01-baseline");
        assert_eq!(scenario_page_slug("S02_canary_1000qps_15min"),    "s02-canary");
        assert_eq!(scenario_page_slug("S03_ramp_to_breakdown"),       "s03-ramp");
        assert_eq!(scenario_page_slug("S04_chaos_participant_loss"),  "s04-chaos-participant-loss");
        assert_eq!(scenario_page_slug("S05_chaos_network_partition"), "s05-chaos-network-partition");
        assert_eq!(scenario_page_slug("S06_chaos_cert_expiry"),       "s06-chaos-cert-expiry");
        assert_eq!(scenario_page_slug("S07_chaos_reload_storm"),      "s07-chaos-reload-storm");
        assert_eq!(scenario_page_slug("S08_multi_region_replay"),     "s08-multi-region");
        assert_eq!(scenario_page_slug("S09_soak_2h_stability"),       "s09-soak");
        assert_eq!(scenario_page_slug("S10_slo_validation_30min"),    "s10-slo-validation");
    }

    #[test]
    fn d_u_page_slug_fallback_for_future_scenarios() {
        // Forward-compat : a PR #25 scenario added without updating
        // SCENARIO_PAGE_FILENAMES still gets a reasonable slug instead
        // of a panic.
        assert_eq!(
            scenario_page_slug("S11_some_new_scenario"),
            "s11-some",
            "unknown scenario must fall back to heuristic"
        );
    }

    #[test]
    fn find_latest_run_handles_missing_dir() {
        let r = find_latest_run(Path::new("/no/such/dir"));
        assert_eq!(r.unwrap(), "");
    }

    #[test]
    fn find_latest_run_picks_latest_creation_and_filters_non_dirs() {
        // Latest-mtime semantics : the dir created last wins (mtimes
        // are NOT guaranteed identical even with quick successive
        // create_dir calls — modern filesystems separate them via
        // nanosecond-resolution timestamps).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("01J3F-aaa")).unwrap();
        std::fs::create_dir(tmp.path().join("01J3F-zzz")).unwrap();
        std::fs::create_dir(tmp.path().join("01J3F-bbb")).unwrap();
        // Non-dir filter check.
        std::fs::write(tmp.path().join("not-a-dir.txt"), "x").unwrap();
        let r = find_latest_run(tmp.path()).unwrap();
        // The honest assertion : the LATEST-CREATED dir wins
        // (here `01J3F-bbb`).  The previous lex-max assertion was a
        // mis-belief that lex sort drove the answer — D-V replaced
        // that with mtime sort.
        assert_eq!(r, "01J3F-bbb");
    }

    #[test]
    #[cfg(unix)]
    fn d_bc_aggregator_handles_unicode_directory_names() {
        // **D-BC** — directory name with non-ASCII chars must round-trip
        // via `to_string_lossy` without losing data, and pass the
        // safety filter (no `/`, `\`, `\0`).
        // Note : Windows path-separator (`\`) is rejected by the filter
        // on every platform — `bench/results/foo\\bar` would already be
        // illegal on POSIX (treated as one literal name) and on
        // Windows is interpreted as a path-traversal.  This test
        // documents that legit Unicode names work.
        let tmp = tempfile::tempdir().unwrap();
        let names = ["run-α", "run-β", "run-中"];
        for n in names {
            std::fs::create_dir(tmp.path().join(n)).unwrap();
        }
        let r = find_latest_run(tmp.path()).unwrap();
        assert!(
            names.contains(&r.as_str()),
            "D-BC : Unicode name should round-trip, got '{r}'"
        );
    }

    #[test]
    fn d_bb_find_latest_run_handles_1000_dirs_quickly() {
        // **D-BB** — 1K run directories must scan in well under 1s.
        // Catches accidental quadratic behaviour (e.g. an O(n²) sort
        // via repeated allocator churn) or syscall storms.
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..1000 {
            std::fs::create_dir(tmp.path().join(format!("01J{i:05}-stress"))).unwrap();
        }
        let start = std::time::Instant::now();
        let r = find_latest_run(tmp.path()).unwrap();
        let elapsed = start.elapsed();
        assert!(
            !r.is_empty(),
            "D-BB : 1K dirs but find_latest_run returned empty"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "D-BB : 1K-dir scan too slow : {elapsed:?}"
        );
    }

    #[test]
    fn d_aj_find_latest_run_rejects_dotfile_directories() {
        // **D-AJ** — hidden / dotfile dirs are never legitimate runs.
        // An operator's `.cache`, `.git`, `.tmp` etc. must not be
        // picked up as fake runs.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("01J3F-real")).unwrap();
        std::fs::create_dir(tmp.path().join(".hidden")).unwrap();
        std::fs::create_dir(tmp.path().join(".cache")).unwrap();
        let r = find_latest_run(tmp.path()).unwrap();
        assert_eq!(
            r, "01J3F-real",
            "D-AJ : dotfile dirs must be filtered out"
        );
    }

    #[test]
    fn d_aj_find_latest_run_empty_when_only_suspicious_entries() {
        // All entries dropped → empty result (and the publisher falls
        // back to NoBaseline rows — no path traversal possible).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".hidden-only")).unwrap();
        std::fs::write(tmp.path().join("a-file.txt"), "x").unwrap();
        let r = find_latest_run(tmp.path()).unwrap();
        assert_eq!(r, "", "D-AJ : no legitimate run → empty string");
    }

    #[test]
    fn d_v_find_latest_run_uses_mtime_not_lex() {
        // **D-V** — when mtimes differ, the actually-most-recent run
        // wins even if it sorts BEFORE lex-max.  Previous implementation
        // used pure lex sort and would have returned "zzz" here.
        use std::time::Duration;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("01J3F-aaa");
        let z = tmp.path().join("01J3F-zzz");
        std::fs::create_dir(&z).unwrap();
        // Sleep long enough to guarantee a different mtime tick
        // (most filesystems are sub-second ; sleep 1100ms to be safe).
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::create_dir(&a).unwrap();
        // Now `a` has a STRICTLY LATER mtime than `z`, despite sorting
        // lex-BEFORE.  D-V demands `a` wins.
        let r = find_latest_run(tmp.path()).unwrap();
        assert_eq!(
            r, "01J3F-aaa",
            "D-V : mtime must dominate lex order"
        );
    }

    #[test]
    fn from_disk_with_no_results_returns_meta_only() {
        let tmp = tempfile::tempdir().unwrap();
        let results = tmp.path().join("results");
        let baselines = tmp.path().join("baselines");
        let scenarios = tmp.path().join("scenarios");
        // Empty bench_results → no badges with data.
        let agg = AggregatedPublish::from_disk(&results, &baselines, &scenarios).unwrap();
        assert!(agg.current_scenarios.is_empty()); // no YAMLs found either
        assert!(agg.badge_rows.is_empty());
        assert_eq!(agg.overall_verdict, Verdict::NoBaseline);
    }

    #[test]
    fn load_baselines_returns_empty_on_missing_dir() {
        let r = load_baselines(Path::new("/no/such/dir")).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn load_baselines_groups_by_chart_version() {
        let tmp = tempfile::tempdir().unwrap();
        let v060 = tmp.path().join("v0.6.0/L2");
        let v070 = tmp.path().join("v0.7.0/L2");
        std::fs::create_dir_all(&v060).unwrap();
        std::fs::create_dir_all(&v070).unwrap();
        let baseline_json = r#"{
            "schema_version": 1,
            "scenario_id": "S01_baseline",
            "harness_version": "0.1.0",
            "loadgen_version": "0.1.0",
            "chart_version": "0.6.0",
            "image_tag": "t",
            "git_commit": "abc",
            "run_id": "r",
            "started_at": "2026-04-01T00:00:00Z",
            "duration_s": 300.0,
            "total_requests": 30000,
            "metrics": {},
            "chaos_events": [],
            "verdict": "pass"
        }"#;
        std::fs::write(v060.join("S01_baseline.json"), baseline_json).unwrap();
        std::fs::write(v070.join("S01_baseline.json"), baseline_json.replace("0.6.0", "0.7.0")).unwrap();
        let r = load_baselines(tmp.path()).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.contains_key("0.6.0"));
        assert!(r.contains_key("0.7.0"));
    }
}
