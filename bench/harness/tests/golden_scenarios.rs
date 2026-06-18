//! PR #21 § 25 — golden test : every committed scenario YAML
//! loads + validates against the harness schema.

use efe_bench_harness::scenario::Scenario;

#[test]
fn every_scenario_loads() {
    let scenarios = Scenario::discover(std::path::Path::new("../../bench/scenarios"))
        .expect("discover scenarios");
    let ids: Vec<_> = scenarios.iter().map(|s| s.id.as_str()).collect();

    // STABILITY_CONTRACT v1.3 § 11.2 : la série **S** est GELÉE à 10 scénarios (PR #21). La série
    // **CB** (control benchmarks, PR D) s'y ajoute SANS toucher au contrat S — on vérifie les deux
    // ensembles séparément (un golden par série), pas un total fragile.
    let s_count = ids.iter().filter(|id| id.starts_with('S')).count();
    assert_eq!(s_count, 10, "la série S est gelée à 10 (STABILITY_CONTRACT v1.3 § 11.2)");

    // IDs S stables (STABILITY_CONTRACT v1.3 § 11.2).
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

    // Série CB (PR D) : 35 control benchmarks, tous chargés + validés (cf. `d_bo` dans engine.rs).
    let cb_count = ids.iter().filter(|id| id.starts_with("CB-")).count();
    assert_eq!(cb_count, 35, "la série CB compte 35 control benchmarks (PR D)");
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
    // Les baselines committées couvrent la série **S** (PR #21). La série **CB** (PR D, control
    // benchmarks) n'a PAS de baselines committées : elles proviendraient de **vrais runs**
    // (concern séparé) — on ne fabrique pas de chiffres. On vérifie donc la correspondance pour S.
    let mut checked = 0;
    for s in scenarios.iter().filter(|s| s.id.starts_with('S')) {
        let baseline_path = baseline_dir.join(format!("{}.json", s.id));
        assert!(
            baseline_path.exists(),
            "missing baseline : {}",
            baseline_path.display()
        );
        checked += 1;
    }
    assert_eq!(checked, 10, "les 10 baselines de la série S doivent toutes exister");
}
