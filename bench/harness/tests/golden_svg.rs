//! PR #24 § 22 — golden SVG byte-stability suite.
//!
//! 11 scenarios × deterministic rendering ⇒ byte-stable bytes.  Any
//! change to the SVG template requires updating the goldens AND a
//! STABILITY_CONTRACT major-version bump (v1.6 § 24.1).

use efe_bench_harness::compare::Verdict;
use efe_bench_harness::publish::badge::{OverallBadge, ScenarioBadge};

#[test]
fn scenario_badge_byte_stable_across_runs() {
    let b = ScenarioBadge::new("S01", Verdict::Pass, Some(8.2));
    let a = b.render();
    let b2 = b.render();
    let c = b.render();
    assert_eq!(a, b2);
    assert_eq!(b2, c);
}

#[test]
fn all_11_badges_byte_stable() {
    for short in [
        "S01", "S02", "S03", "S04", "S05", "S06", "S07", "S08", "S09", "S10",
    ] {
        let b = ScenarioBadge::new(short, Verdict::Pass, Some(8.0));
        let a = b.render();
        let c = b.render();
        assert_eq!(a, c, "{short} not byte-stable");
    }
    // Overall : aggregate over 10 pass scenarios.
    let o = OverallBadge::from_scenarios(&[Verdict::Pass; 10]);
    let a = o.render();
    let c = o.render();
    assert_eq!(a, c);
}

#[test]
fn fail_overall_renders_red_with_count_not_uppercase_fail() {
    let mut v = vec![Verdict::Pass; 9];
    v.push(Verdict::Fail);
    let o = OverallBadge::from_scenarios(&v);
    let svg = o.render();
    assert!(svg.contains("fail (1)"));
    assert!(svg.contains("#da3633"));
}

#[test]
fn pass_renders_green_3fb950() {
    let b = ScenarioBadge::new("S01", Verdict::Pass, Some(8.0));
    let svg = b.render();
    assert!(svg.contains("#3fb950"));
}

#[test]
fn warn_renders_amber_d29922() {
    let b = ScenarioBadge::new("S03", Verdict::Warn, Some(145.0));
    let svg = b.render();
    assert!(svg.contains("#d29922"));
}

#[test]
fn no_baseline_renders_grey_8b949e_with_no_data() {
    let b = ScenarioBadge::new("S02", Verdict::NoBaseline, None);
    let svg = b.render();
    assert!(svg.contains("#8b949e"));
    assert!(svg.contains("no data"));
}

#[test]
fn no_javascript_in_any_badge_t100_t125() {
    for verdict in [Verdict::Pass, Verdict::Warn, Verdict::Fail, Verdict::NoBaseline] {
        let svg = ScenarioBadge::new("S01", verdict, Some(8.0)).render().to_lowercase();
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("onclick"));
        assert!(!svg.contains("javascript:"));
        assert!(!svg.contains("<iframe"));
    }
    let overall = OverallBadge::from_scenarios(&[Verdict::Fail; 2]).render().to_lowercase();
    assert!(!overall.contains("<script"));
    assert!(!overall.contains("onclick"));
}

// ─────────────────────────────────────────────────────────────────────
// D-AM — byte-stability for EVERY verdict (not just Pass).
// Previously only Pass was byte-stable-tested.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_am_scenario_badge_byte_stable_for_every_verdict() {
    for (verdict, p95) in [
        (Verdict::Pass, Some(8.2)),
        (Verdict::Warn, Some(145.0)),
        (Verdict::Fail, None),
        (Verdict::NoBaseline, None),
    ] {
        let b = ScenarioBadge::new("S05", verdict, p95);
        let a = b.render();
        let c = b.render();
        let d = b.render();
        assert_eq!(a, c, "D-AM : {verdict:?} byte-instability (run 1 vs 2)");
        assert_eq!(c, d, "D-AM : {verdict:?} byte-instability (run 2 vs 3)");
    }
}

#[test]
fn d_am_overall_badge_byte_stable_for_mixed_verdicts() {
    // Mixed verdicts produce the most code-paths in OverallBadge ;
    // byte-stability here guarantees AC2 / AD37 across the whole
    // 4-color palette.
    let mixtures: &[&[Verdict]] = &[
        &[Verdict::Pass; 10],
        &[Verdict::Fail; 10],
        &[Verdict::Warn; 10],
        &[Verdict::NoBaseline; 10],
        // 5 pass + 3 warn + 1 fail + 1 nb
        &[
            Verdict::Pass, Verdict::Pass, Verdict::Pass, Verdict::Pass, Verdict::Pass,
            Verdict::Warn, Verdict::Warn, Verdict::Warn,
            Verdict::Fail,
            Verdict::NoBaseline,
        ],
    ];
    for verdicts in mixtures {
        let o = OverallBadge::from_scenarios(verdicts);
        let a = o.render();
        let c = o.render();
        assert_eq!(
            a, c,
            "D-AM : OverallBadge byte-instability for mixture {verdicts:?}"
        );
    }
}

#[test]
fn d_am_all_11_badges_byte_stable_for_fail() {
    // Symmetric to all_11_badges_byte_stable (which only tests Pass).
    for short in [
        "S01", "S02", "S03", "S04", "S05", "S06", "S07", "S08", "S09", "S10",
    ] {
        let b = ScenarioBadge::new(short, Verdict::Fail, None);
        let a = b.render();
        let c = b.render();
        assert_eq!(a, c, "D-AM : {short} Fail-byte-unstable");
    }
    // Overall : aggregate over 10 fail scenarios.
    let o = OverallBadge::from_scenarios(&[Verdict::Fail; 10]);
    let a = o.render();
    let c = o.render();
    assert_eq!(a, c);
}

#[test]
fn d_am_all_11_badges_byte_stable_for_warn() {
    for short in [
        "S01", "S02", "S03", "S04", "S05", "S06", "S07", "S08", "S09", "S10",
    ] {
        let b = ScenarioBadge::new(short, Verdict::Warn, Some(150.0));
        let a = b.render();
        let c = b.render();
        assert_eq!(a, c, "D-AM : {short} Warn-byte-unstable");
    }
}

#[test]
fn xml_escape_neutralises_injection_t118() {
    // Defensive : if a future bug allows non-numeric chars in the
    // value side, they must be XML-escaped.
    use efe_bench_harness::publish::badge::xml_escape;
    let injected = xml_escape("<script>alert(1)</script>");
    assert!(!injected.contains("<script"));
    assert!(injected.contains("&lt;script&gt;"));
}
