//! PR #22 § 25 — property-based tests for window alignment + boundary.

use efe_slo_rollup::window::WindowSpec;
use proptest::prelude::*;

proptest! {
    /// `align` is idempotent : align(align(x)) == align(x).
    #[test]
    fn align_idempotent(x in i64::MIN/2..i64::MAX/2) {
        for w in WindowSpec::all() {
            let a = w.align(x);
            prop_assert_eq!(w.align(a), a);
        }
    }

    /// `align` always floors : align(x) <= x.
    #[test]
    fn align_floors(x in 0i64..3_000_000_000) {
        for w in WindowSpec::all() {
            let a = w.align(x);
            prop_assert!(a <= x);
        }
    }

    /// align(x) is congruent to 0 mod window_size.
    #[test]
    fn align_is_multiple_of_window_size(x in 0i64..3_000_000_000) {
        for w in WindowSpec::all() {
            let a = w.align(x);
            prop_assert_eq!(a % (w.as_seconds() as i64), 0);
        }
    }

    /// Parse(as_str(w)) == Some(w) for all windows.
    #[test]
    fn string_form_roundtrips(idx in 0usize..8) {
        let w = WindowSpec::all()[idx];
        prop_assert_eq!(WindowSpec::parse(w.as_str()), Some(w));
    }
}

#[test]
fn five_min_aligns_clockwise() {
    // 12:34:56 → 12:30:00
    let ts: i64 = 12 * 3600 + 34 * 60 + 56;
    let a = WindowSpec::FiveMin.align(ts);
    assert_eq!(a, 12 * 3600 + 30 * 60);
}

#[test]
fn hour_aligns_to_hour_boundary() {
    let ts: i64 = 3 * 3600 + 45 * 60;
    let a = WindowSpec::OneHour.align(ts);
    assert_eq!(a, 3 * 3600);
}

#[test]
fn day_aligns_to_utc_midnight() {
    let date = "2026-05-15";
    let unix = efe_slo_rollup::window::align::parse_date_to_unix(date).unwrap();
    let ts_3am = unix + 3 * 3600;
    let aligned = WindowSpec::TwentyFourHours.align(ts_3am);
    assert_eq!(aligned, unix);
}
