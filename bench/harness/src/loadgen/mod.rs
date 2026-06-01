//! PR #21 § 18 — PR #20 loadgen integration.
//!
//! Two paths :
//!   * **In-process library mode** (default — AD2) : import
//!     `efe_meta_swarm_loadgen` as a library and call `run(config)`.
//!   * **Subprocess mode** (`--loadgen-subprocess`) : spawn the binary
//!     as a child process (fallback for distroless dev environments
//!     without Cargo).

use std::time::Duration;

use crate::error::{BenchError, BenchResult};
use crate::result::ResultMetrics;
use crate::scenario::Scenario;

/// Convert a scenario's embedded loadgen YAML into a working
/// [`efe_meta_swarm_loadgen::LoadgenConfig`].
///
/// The scenario's `loadgen:` block is re-serialised to a temp YAML
/// file, then loaded via the loadgen profile loader.  This keeps the
/// schema versioning + production-mode refusal in one place.
///
/// If `target_override` is `Some(url)`, every endpoint's `url` field
/// inside `loadgen.target.endpoints[]` is rewritten to that URL
/// (weight + role preserved).  Used by CI workflows to redirect
/// in-cluster scenario targets to a `kubectl port-forward` localhost
/// proxy (PR #21 § 14 — G1/G2/G6 closure).
pub async fn run_loadgen_for_scenario(
    scenario: &Scenario,
    target_override: Option<&str>,
) -> BenchResult<ResultMetrics> {
    use efe_meta_swarm_loadgen::profile::Profile;
    use efe_meta_swarm_loadgen::{run as loadgen_run, LoadgenConfig};

    // ── 0.  Apply target override (CI escape hatch) before re-serialising.
    let mut scenario_owned = scenario.clone();
    if let Some(url) = target_override {
        rewrite_endpoint_urls(&mut scenario_owned, url)?;
    }
    let scenario = &scenario_owned;

    // ── 1.  Re-emit the embedded profile to a temp YAML.
    let profile_yaml = profile_yaml_from_scenario(scenario)?;
    let tmp = tempfile::NamedTempFile::new()
        .map_err(|e| BenchError::Loadgen(format!("tempfile : {e}")))?;
    std::fs::write(tmp.path(), profile_yaml)
        .map_err(|e| BenchError::Loadgen(format!("write profile : {e}")))?;

    // ── 2.  Load via the PR #20 profile parser (re-validates schema).
    let profile = Profile::load(tmp.path())
        .map_err(|e| BenchError::Loadgen(format!("loadgen profile reject : {e}")))?;

    // ── 3.  Build a minimal RunArgs + merge with the profile.
    //         The bench harness only needs the targets + workload +
    //         pattern from the profile ; we don't pass any CLI flags.
    let args = build_minimal_run_args(scenario)?;
    let config = LoadgenConfig::from_cli_and_profile(args, Some(profile))
        .map_err(|e| BenchError::Loadgen(format!("loadgen config : {e}")))?;

    // ── 4.  Run the loadgen library.
    let summary = loadgen_run(config)
        .await
        .map_err(|e| BenchError::Loadgen(format!("loadgen run : {e}")))?;

    // ── 5.  Convert RunSummary → ResultMetrics.
    Ok(ResultMetrics {
        p50_ms: Some(summary.p50 * 1000.0),
        p95_ms: Some(summary.p95 * 1000.0),
        p99_ms: Some(summary.p99 * 1000.0),
        error_rate: Some(summary.error_rate),
        throughput_qps: if summary.duration_s > 0.0 {
            Some(summary.total_requests as f64 / summary.duration_s)
        } else {
            None
        },
        // Recovery time is computed by the chaos applier (Phase-3) —
        // not surfaced by loadgen.
        recovery_time_s: None,
    })
}

/// Re-serialise the scenario's `loadgen:` block as a standalone profile YAML.
fn profile_yaml_from_scenario(scenario: &Scenario) -> BenchResult<String> {
    serde_yaml::to_string(&scenario.loadgen)
        .map_err(|e| BenchError::Loadgen(format!("yaml serialize : {e}")))
}

/// Build a minimal [`efe_meta_swarm_loadgen::cli::RunArgs`] without
/// any CLI knobs ; the profile carries everything we need.
fn build_minimal_run_args(
    scenario: &Scenario,
) -> BenchResult<efe_meta_swarm_loadgen::cli::RunArgs> {
    // We rely on the profile YAML for the heavy lifting ; the CLI
    // shape mostly forces mode + dry_run.
    let args = efe_meta_swarm_loadgen::cli::RunArgs {
        target: vec![],
        target_role: efe_meta_swarm_loadgen::cli::TargetRole::Controller,
        conns_per_target: 4,
        per_request_deadline: None,
        profile: None,  // already loaded inline
        pattern: efe_meta_swarm_loadgen::cli::PatternKind::Constant,
        workload: efe_meta_swarm_loadgen::cli::WorkloadKind::Iterate,
        workload_mix: None,
        qps: 10.0,
        concurrency: None,
        rate_mode: efe_meta_swarm_loadgen::cli::RateMode::Hybrid,
        ramp_up: Duration::from_secs(0),
        duration: Some(scenario.duration()?),
        requests: None,
        until_slo: vec![],
        shutdown_deadline: Duration::from_secs(5),
        tls_cert: None,
        tls_key: None,
        tls_ca: None,
        expected_san: None,
        tls_insecure: false,
        prom_listen: "0.0.0.0:9095".into(),
        otel_endpoint: None,
        otel_sampler: 0.0,
        otel_service_name: "efe-bench-harness-loadgen".into(),
        output_format: efe_meta_swarm_loadgen::cli::OutputFormat::Json,
        output_file: None,
        record: false,
        seed: None,
        mode: efe_meta_swarm_loadgen::cli::RunMode::Dev,
        confirm_prod: false,
        dry_run: false,
        allow_mutation: false,
        i_am_sure: false,
        chaos_marker: None,
        wait_for_signal: None,
    };
    Ok(args)
}

/// Rewrite every endpoint URL inside `scenario.loadgen.target.endpoints[]`
/// to `new_url`.  Other fields (weight, role, conns_per_target, per_request_deadline)
/// are preserved.  Returns an error if the YAML shape doesn't have an
/// `endpoints` sequence (catches malformed scenarios early).
fn rewrite_endpoint_urls(scenario: &mut Scenario, new_url: &str) -> BenchResult<()> {
    let target = &mut scenario.loadgen.target;
    let map = target.as_mapping_mut().ok_or_else(|| {
        BenchError::Loadgen(format!(
            "scenario {} : loadgen.target is not a YAML mapping (cannot apply --target-override)",
            scenario.id
        ))
    })?;
    let eps = map
        .get_mut(serde_yaml::Value::String("endpoints".into()))
        .and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| {
            BenchError::Loadgen(format!(
                "scenario {} : loadgen.target.endpoints is missing or not a sequence",
                scenario.id
            ))
        })?;
    if eps.is_empty() {
        return Err(BenchError::Loadgen(format!(
            "scenario {} : loadgen.target.endpoints is empty",
            scenario.id
        )));
    }
    for ep in eps.iter_mut() {
        if let Some(ep_map) = ep.as_mapping_mut() {
            ep_map.insert(
                serde_yaml::Value::String("url".into()),
                serde_yaml::Value::String(new_url.into()),
            );
        } else {
            return Err(BenchError::Loadgen(format!(
                "scenario {} : endpoint is not a mapping",
                scenario.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ScenarioLoadgen;

    fn make_scenario() -> Scenario {
        Scenario {
            schema_version: 1,
            id: "S01_test".into(),
            description: "t".into(),
            duration: "1s".into(),
            cluster: Default::default(),
            loadgen: ScenarioLoadgen {
                version: 1,
                name: "t".into(),
                target: serde_yaml::from_str(
                    "endpoints:\n  - url: grpc://localhost:50051",
                ).unwrap(),
                pattern: serde_yaml::from_str("type: constant\nqps: 10").unwrap(),
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
        }
    }

    #[test]
    fn profile_yaml_roundtrips() {
        let s = make_scenario();
        let yaml = profile_yaml_from_scenario(&s).unwrap();
        assert!(yaml.contains("name: t"));
        assert!(yaml.contains("grpc://localhost:50051"));
    }

    #[test]
    fn target_override_rewrites_every_endpoint() {
        let mut s = make_scenario();
        // Inject two endpoints so we verify *every* one is rewritten.
        s.loadgen.target = serde_yaml::from_str(
            "endpoints:\n  - { url: grpc://a:50051, weight: 0.5 }\n  - { url: grpc://b:50051, weight: 0.5 }",
        ).unwrap();
        rewrite_endpoint_urls(&mut s, "grpc://localhost:50051").unwrap();
        let yaml = serde_yaml::to_string(&s.loadgen.target).unwrap();
        assert!(yaml.contains("grpc://localhost:50051"));
        assert!(!yaml.contains("grpc://a:50051"));
        assert!(!yaml.contains("grpc://b:50051"));
        // Weights must be preserved.
        assert!(yaml.contains("weight: 0.5"));
    }

    #[test]
    fn target_override_rejects_empty_endpoints() {
        let mut s = make_scenario();
        s.loadgen.target = serde_yaml::from_str("endpoints: []").unwrap();
        let e = rewrite_endpoint_urls(&mut s, "grpc://x:1").unwrap_err();
        assert!(matches!(e, BenchError::Loadgen(_)));
    }

    #[test]
    fn target_override_rejects_non_mapping_target() {
        let mut s = make_scenario();
        s.loadgen.target = serde_yaml::Value::String("not-a-mapping".into());
        let e = rewrite_endpoint_urls(&mut s, "grpc://x:1").unwrap_err();
        assert!(matches!(e, BenchError::Loadgen(_)));
    }
}
