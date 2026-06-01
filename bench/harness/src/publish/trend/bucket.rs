//! PR #24 § 9 + AD11 — day-bucketing logic for the 30-day sparkline.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::publish::trend::DailyPoint;

/// Given a list of `(timestamp_unix, p95_ms)` samples, bucket them into
/// 30 UTC days ending at `today` (exclusive of `today + 1 day`).
/// For days with multiple samples, the **last** sample wins (AD11 —
/// "the most recent run in that day's window").
///
/// Days with no sample yield `DailyPoint { p95_ms: None }`.
#[must_use]
pub fn bucket_30_days(samples: &[(i64, f64)], today_utc_unix: i64, window_days: u32) -> Vec<DailyPoint> {
    // Determine the start-of-window day (today - (window_days - 1)).
    let today = DateTime::<Utc>::from_timestamp(today_utc_unix, 0)
        .map(|d| d.date_naive())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());

    let start = today - chrono::Duration::days((window_days as i64).saturating_sub(1));

    // Bucket samples by their UTC date.  Last-sample-wins per day.
    let mut by_day: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for (ts, p95) in samples {
        if let Some(dt) = DateTime::<Utc>::from_timestamp(*ts, 0) {
            let day = dt.date_naive();
            if day >= start && day <= today {
                by_day.insert(day, *p95); // overwrite — last wins
            }
        }
    }

    // Emit one DailyPoint per day in the window.
    (0..window_days)
        .map(|i| {
            let day = start + chrono::Duration::days(i as i64);
            let day_index = day_unix_index(day);
            DailyPoint {
                day_index,
                p95_ms: by_day.get(&day).copied(),
            }
        })
        .collect()
}

/// Convert a `NaiveDate` to the unix-days-since-epoch index used by
/// [`DailyPoint::day_index`].
fn day_unix_index(d: NaiveDate) -> u32 {
    let dt = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap());
    (dt.timestamp() / 86_400) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_for(date: &str) -> i64 {
        let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        Utc.from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
            .timestamp()
    }

    #[test]
    fn bucket_30_days_returns_exactly_30() {
        let today = unix_for("2026-05-30");
        let samples = vec![
            (unix_for("2026-05-29"), 8.2),
            (unix_for("2026-05-30"), 9.1),
        ];
        let buckets = bucket_30_days(&samples, today, 30);
        assert_eq!(buckets.len(), 30);
    }

    #[test]
    fn last_sample_wins_per_day() {
        let today = unix_for("2026-05-30");
        let samples = vec![
            (unix_for("2026-05-30"), 8.2), // earlier
            (unix_for("2026-05-30"), 12.5), // later — wins (BTreeMap insert overwrites)
        ];
        let buckets = bucket_30_days(&samples, today, 30);
        let today_bucket = buckets.last().unwrap();
        assert_eq!(today_bucket.p95_ms, Some(12.5));
    }

    #[test]
    fn samples_outside_window_excluded() {
        let today = unix_for("2026-05-30");
        let samples = vec![
            (unix_for("2026-04-15"), 50.0), // 45 days ago — outside 30-day window
            (unix_for("2026-05-30"), 9.0),
        ];
        let buckets = bucket_30_days(&samples, today, 30);
        let in_window: usize = buckets.iter().filter(|p| p.p95_ms.is_some()).count();
        assert_eq!(in_window, 1);
    }

    #[test]
    fn all_missing_days_yield_none() {
        let today = unix_for("2026-05-30");
        let buckets = bucket_30_days(&[], today, 30);
        for p in &buckets {
            assert!(p.p95_ms.is_none());
        }
    }

    #[test]
    fn day_indices_are_strictly_increasing() {
        let today = unix_for("2026-05-30");
        let buckets = bucket_30_days(&[], today, 30);
        for w in buckets.windows(2) {
            assert!(w[0].day_index < w[1].day_index);
        }
    }

    #[test]
    fn supports_custom_window_size() {
        let today = unix_for("2026-05-30");
        let buckets = bucket_30_days(&[], today, 7);
        assert_eq!(buckets.len(), 7);
    }
}
