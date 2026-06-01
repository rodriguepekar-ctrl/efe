//! PR #23 § 21 + AD28 — badge SVG byte-stability golden suite.
//!
//! Each render call MUST be byte-identical across runs (no time, no
//! UUID, no random in the output).  T100 — no script / onclick / JS.

use efe_smoke::checks::CheckVerdict;
use efe_smoke::result::badge::render;

#[test]
fn pass_badge_is_byte_stable_across_runs() {
    let a = render(CheckVerdict::Pass, 0);
    let b = render(CheckVerdict::Pass, 0);
    let c = render(CheckVerdict::Pass, 0);
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn fail_badge_is_byte_stable_across_runs() {
    let a = render(CheckVerdict::Fail, 3);
    let b = render(CheckVerdict::Fail, 3);
    assert_eq!(a, b);
}

#[test]
fn pass_and_fail_render_to_different_bytes() {
    let pass = render(CheckVerdict::Pass, 0);
    let fail = render(CheckVerdict::Fail, 3);
    assert_ne!(pass, fail);
}

#[test]
fn no_javascript_anywhere_t100() {
    for (verdict, count) in [
        (CheckVerdict::Pass, 0),
        (CheckVerdict::Pass, 9),
        (CheckVerdict::Fail, 0),
        (CheckVerdict::Fail, 1),
        (CheckVerdict::Fail, 9),
    ] {
        let svg = render(verdict, count).to_lowercase();
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("onclick"));
        assert!(!svg.contains("javascript:"));
        assert!(!svg.contains("onload"));
    }
}

#[test]
fn badge_starts_with_xml_decl_and_ends_with_svg_close() {
    let s = render(CheckVerdict::Pass, 0);
    assert!(s.starts_with("<?xml"));
    assert!(s.ends_with("</svg>"));
}
