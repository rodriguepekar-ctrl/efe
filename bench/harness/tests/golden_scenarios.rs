//! PR #21 § 25 — golden test : every committed scenario YAML
//! loads + validates against the harness schema.

use efe_bench_harness::scenario::Scenario;

#[test]
fn every_scenario_loads() {
    let scenarios = Scenario::discover(std::path::Path::new("../../bench/scenarios"))
        .expect("discover scenarios");
    assert_eq!(scenarios.len(), 10, "PR #21 commits 10 scenarios (STABILITY_CONTRACT v1.3 § 11.2)");

    // Stable IDs (STABILITY_CONTRACT v1.3 § 11.2).
    let ids: Vec<_> = scenarios.iter().map(|s| s.id.as_str()).collect();
    let expected = [
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
    ];
    for id in expected {
        assert!(ids.contains(&id), "missing scenario : {id}");
    }
}

#[test]
fn chaos_scenarios_reference_pr21_crds() {
    let scenarios = Scenario::discover(std::path::Path::new("../../bench/scenarios"))
        .expect("discover");
    for s in &scenarios {
        for fault in &s.chaos {
            if let Some(file) = &fault.crd_file {
                assert!(
                    file.starts_with("pr21_meta_swarm/"),
                    "scenario {} chaos fault {} must reference pr21_meta_swarm/ ; got {}",
                    s.id, fault.id, file,
                );
            }
        }
    }
}

#[test]
fn scenarios_with_chaos_are_s04_through_s07() {
    let scenarios = Scenario::discover(std::path::Path::new("../../bench/scenarios"))
        .expect("discover");
    let chaos_ids: Vec<_> = scenarios
        .iter()
        .filter(|s| !s.chaos.is_empty())
        .map(|s| s.id.clone())
        .collect();
    // S04, S05, S06, S07 are the chaos scenarios per the arch doc.
    assert!(chaos_ids.contains(&"S04_chaos_participant_loss".to_string()));
    assert!(chaos_ids.contains(&"S05_chaos_network_partition".to_string()));
    assert!(chaos_ids.contains(&"S06_chaos_cert_expiry".to_string()));
    assert!(chaos_ids.contains(&"S07_chaos_reload_storm".to_string()));
    assert_eq!(chaos_ids.len(), 4, "expected exactly 4 chaos scenarios");
}

#[test]
fn baseline_files_match_scenarios() {
    let scenarios = Scenario::discover(std::path::Path::new("../../bench/scenarios"))
        .expect("discover");
    let baseline_dir = std::path::Path::new("../../bench_baselines/v0.6.0/L2");
    for s in &scenarios {
        let baseline_path = baseline_dir.join(format!("{}.json", s.id));
        assert!(
            baseline_path.exists(),
            "missing baseline : {}",
            baseline_path.display()
        );
    }
}
