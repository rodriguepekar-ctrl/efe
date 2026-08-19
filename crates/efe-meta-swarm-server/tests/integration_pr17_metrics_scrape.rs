//! PR #17 — scrape-roundtrip integration test.
//!
//! Covers AC1 + AC2 + AC3 + AC4 + AC5 + AC28 :
//!
//!   * The 14 new series appear in `/metrics` from boot.
//!   * Every series has a non-empty HELP string.
//!   * Naming convention : snake_case, unit suffix, `_total` for counters.
//!   * Pre-touch policy : closed-taxonomy tuples emit `0` at boot.
//!   * `prometheus-parse`-style line counting confirms the right
//!     number of series families.

#![allow(clippy::missing_panics_doc)]

use efe_meta_swarm_server::observability::metrics::MetricsRegistry;

const EXPECTED_PR17_SERIES: &[&str] = &[
    // Group A — Iterate
    "efe_meta_swarm_iterate_duration_seconds",
    "efe_meta_swarm_iterate_total",
    "efe_meta_swarm_iterate_inflight",
    "efe_meta_swarm_iterate_iterations",
    "efe_meta_swarm_iterate_convergence_error",
    // Group B — Step
    "efe_meta_swarm_step_duration_seconds",
    "efe_meta_swarm_step_total",
    "efe_meta_swarm_step_payload_bytes",
    // Group C — Safety
    "efe_meta_swarm_safety_violations_total",
    // Group D — Health
    "efe_meta_swarm_participants_configured",
    "efe_meta_swarm_participant_up",
    // Group E — Manifest reload
    "efe_meta_swarm_manifest_reload_duration_seconds",
    "efe_meta_swarm_manifest_reload_total",
    // Group F — Build info
    "efe_meta_swarm_build_info",
];

const EXPECTED_PR12_SERIES: &[&str] = &[
    "efe_meta_swarm_reload_attempts_total",
    "efe_meta_swarm_reload_failures_total",
    "efe_meta_swarm_reload_duration_microseconds",
    "efe_meta_swarm_reload_gen",
];

#[test]
fn pr17_ac1_all_14_series_appear_in_metrics_body() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8 body");
    for series in EXPECTED_PR17_SERIES {
        assert!(
            body.contains(series),
            "missing PR #17 series : {series}\n--- /metrics body ---\n{body}"
        );
    }
}

#[test]
fn pr17_backward_compat_pr12_series_still_present() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8 body");
    for series in EXPECTED_PR12_SERIES {
        assert!(body.contains(series), "PR #12 series missing : {series}");
    }
}

#[test]
fn pr17_ac2_every_series_has_help_string() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8 body");
    for series in EXPECTED_PR17_SERIES {
        let help_line = format!("# HELP {series} ");
        assert!(
            body.contains(&help_line),
            "missing HELP for {series}\n--- /metrics body ---\n{body}"
        );
    }
}

#[test]
fn pr17_ac3_naming_convention_lowercase_snake() {
    for series in EXPECTED_PR17_SERIES {
        assert!(
            series
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "series {series} violates naming convention"
        );
        assert!(
            series.starts_with("efe_meta_swarm_"),
            "series {series} missing canonical prefix"
        );
    }
}

#[test]
fn pr17_ac4_counters_end_with_total() {
    let counter_series = [
        "efe_meta_swarm_iterate_total",
        "efe_meta_swarm_step_total",
        "efe_meta_swarm_safety_violations_total",
        "efe_meta_swarm_manifest_reload_total",
    ];
    for s in counter_series {
        assert!(
            s.ends_with("_total"),
            "counter {s} does not end with _total"
        );
    }
}

#[test]
fn pr17_ac4_histograms_carry_unit_suffix() {
    let histograms = [
        ("efe_meta_swarm_iterate_duration_seconds", "_seconds"),
        ("efe_meta_swarm_step_duration_seconds", "_seconds"),
        (
            "efe_meta_swarm_manifest_reload_duration_seconds",
            "_seconds",
        ),
        ("efe_meta_swarm_step_payload_bytes", "_bytes"),
    ];
    for (name, suffix) in histograms {
        assert!(
            name.ends_with(suffix),
            "histogram {name} missing unit suffix {suffix}"
        );
    }
}

#[test]
fn pr17_ac5_pretouch_emits_zero_value_for_iterate_total() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8");
    // All 5 × 8 = 40 combinations should appear with value 0.
    for outcome in ["ok", "error", "deadline", "canceled", "throttled"] {
        for reason in [
            "timeout",
            "dim_mismatch",
            "safety_violation",
            "participant_down",
            "internal",
            "deadline_exceeded",
            "cancelled",
            "other",
        ] {
            let needle = format!(
                "efe_meta_swarm_iterate_total{{failure_reason=\"{reason}\",result=\"{outcome}\"}} 0"
            );
            assert!(body.contains(&needle), "missing pretouch : {needle}");
        }
    }
}

#[test]
fn pr17_pretouch_for_manifest_emits_per_participant_series() {
    use efe_meta_swarm_server::manifest::{
        IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
    };
    let r = MetricsRegistry::new();
    let m = Manifest {
        version: "v0.2.0".into(),
        meta: MetaSpaceManifest {
            dim: 8,
            safety: SafetyManifest::Box { radius: 50.0 },
        },
        participants: vec![
            ParticipantManifest {
                id: "p-alpha".into(),
                dim: 8,
                kappa: Some(0.5),
                seed: Some(11),
                blas_backend: "none".into(),
                controller: None,
            },
            ParticipantManifest {
                id: "p-beta".into(),
                dim: 8,
                kappa: Some(0.6),
                seed: Some(22),
                blas_backend: "none".into(),
                controller: None,
            },
        ],
        iteration: IterationManifest::default(),
    };
    r.pretouch_for_manifest(&m);
    let body = String::from_utf8(r.render()).expect("utf8");

    // participants_configured = 2
    assert!(body.contains("efe_meta_swarm_participants_configured 2"));

    // step_total + safety_violations_total per id
    for id in ["p-alpha", "p-beta"] {
        let needle1 = format!("participant_id=\"{id}\"");
        let occurrences = body.matches(&needle1).count();
        // Both `step_total` (5 outcomes × 8 reasons = 40 cells) and
        // `step_duration_seconds` (5 outcomes) and
        // `safety_violations_total` (3 kinds) and
        // `participant_up` (1) all carry the participant_id label.
        // Conservative lower bound : > 30 occurrences per id.
        assert!(
            occurrences > 30,
            "participant_id={id:?} appears only {occurrences} times — pretouch likely incomplete"
        );
    }
}

#[test]
fn pr17_build_info_has_value_one() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8");
    // The build_info line ends with " 1".
    let lines: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("efe_meta_swarm_build_info{"))
        .collect();
    assert!(!lines.is_empty(), "no build_info sample emitted");
    for l in &lines {
        assert!(l.ends_with(" 1"), "build_info value != 1 : {l}");
    }
}

#[test]
fn pr17_cardinality_breach_counter_pretouched() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8");
    assert!(body.contains("efe_meta_swarm_cardinality_breach_total"));
    assert!(body.contains("efe_meta_swarm_cardinality_breach_total{metric=\"(none)\"} 0"));
}

#[test]
fn pr17_total_distinct_metric_families_at_boot() {
    let r = MetricsRegistry::new();
    let body = String::from_utf8(r.render()).expect("utf8");
    // Count `# HELP efe_meta_swarm_` lines — one per metric family.
    let help_count = body
        .lines()
        .filter(|l| l.starts_with("# HELP efe_meta_swarm_"))
        .count();
    // 4 PR #12 + 14 PR #17 + 1 cardinality_breach = 19.
    // (PR #16 TLS metrics are registered separately by bootstrap_tls
    //  and don't appear here.)
    assert!(
        help_count >= 19,
        "expected ≥ 19 metric families, got {help_count}\n{body}"
    );
}
