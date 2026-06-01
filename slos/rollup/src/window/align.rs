//! PR #22 § 8.1 — clock-aligned bucket math (UTC only — AD8 + AD20).

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};

/// Format a unix-seconds boundary as `2026-05-15T03:00:00Z`.
#[must_use]
pub fn fmt_iso(unix_secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "INVALID".into())
}

/// Format a unix-seconds boundary as a date string `YYYY-MM-DD`.
#[must_use]
pub fn fmt_date(unix_secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "INVALID".into())
}

/// Format a unix-seconds boundary as a month string `YYYY-MM`.
#[must_use]
pub fn fmt_month(unix_secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .map(|d| d.format("%Y-%m").to_string())
        .unwrap_or_else(|| "INVALID".into())
}

/// Determine the quarter (1-4) of a unix-seconds boundary (UTC).
#[must_use]
pub fn quarter_of(unix_secs: i64) -> u32 {
    DateTime::<Utc>::from_timestamp(unix_secs, 0)
        .map(|d| (d.month() - 1) / 3 + 1)
        .unwrap_or(0)
}

/// Format as `Q2-2026`.
#[must_use]
pub fn fmt_quarter(unix_secs: i64) -> String {
    let q = quarter_of(unix_secs);
    let y = DateTime::<Utc>::from_timestamp(unix_secs, 0).map(|d| d.year()).unwrap_or(0);
    format!("Q{q}-{y}")
}

/// Parse a `YYYY-MM-DD` into the UTC unix-seconds of midnight that day.
pub fn parse_date_to_unix(date_str: &str) -> Option<i64> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| Utc.from_utc_datetime(&d).timestamp())
}

/// Parse a `YYYY-MM` into the UTC unix-seconds of the 1st of that month.
pub fn parse_month_to_unix(month_str: &str) -> Option<i64> {
    let s = format!("{month_str}-01");
    parse_date_to_unix(&s)
}

/// Parse `Q{1,2,3,4}-{YYYY}` into the unix-seconds of the 1st day of
/// that quarter.
pub fn parse_quarter_to_unix(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 7 || bytes[0] != b'Q' || bytes[2] != b'-' {
        return None;
    }
    let q: u32 = std::str::from_utf8(&bytes[1..2]).ok()?.parse().ok()?;
    let year: i32 = std::str::from_utf8(&bytes[3..]).ok()?.parse().ok()?;
    let first_month = match q {
        1 => 1,
        2 => 4,
        3 => 7,
        4 => 10,
        _ => return None,
    };
    let d = NaiveDate::from_ymd_opt(year, first_month, 1)?.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&d).timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_format() {
        // 2026-05-15T03:00:00Z = unix 1779667200
        let ts = parse_date_to_unix("2026-05-15").unwrap() + 3 * 3600;
        assert_eq!(fmt_iso(ts), "2026-05-15T03:00:00Z");
    }

    #[test]
    fn date_format() {
        let ts = parse_date_to_unix("2026-05-15").unwrap();
        assert_eq!(fmt_date(ts), "2026-05-15");
    }

    #[test]
    fn month_format() {
        let ts = parse_date_to_unix("2026-05-15").unwrap();
        assert_eq!(fmt_month(ts), "2026-05");
    }

    #[test]
    fn quarter_boundaries() {
        assert_eq!(quarter_of(parse_date_to_unix("2026-01-15").unwrap()), 1);
        assert_eq!(quarter_of(parse_date_to_unix("2026-03-31").unwrap()), 1);
        assert_eq!(quarter_of(parse_date_to_unix("2026-04-01").unwrap()), 2);
        assert_eq!(quarter_of(parse_date_to_unix("2026-06-30").unwrap()), 2);
        assert_eq!(quarter_of(parse_date_to_unix("2026-07-01").unwrap()), 3);
        assert_eq!(quarter_of(parse_date_to_unix("2026-10-15").unwrap()), 4);
        assert_eq!(quarter_of(parse_date_to_unix("2026-12-31").unwrap()), 4);
    }

    #[test]
    fn fmt_quarter_canonical() {
        let ts = parse_date_to_unix("2026-05-15").unwrap();
        assert_eq!(fmt_quarter(ts), "Q2-2026");
    }

    #[test]
    fn parse_quarter_roundtrips() {
        let ts = parse_quarter_to_unix("Q2-2026").unwrap();
        assert_eq!(fmt_quarter(ts), "Q2-2026");
        assert_eq!(fmt_date(ts), "2026-04-01");
    }

    #[test]
    fn parse_quarter_rejects_garbage() {
        assert!(parse_quarter_to_unix("X1-2026").is_none());
        assert!(parse_quarter_to_unix("Q5-2026").is_none());
        assert!(parse_quarter_to_unix("Q2_2026").is_none());
        assert!(parse_quarter_to_unix("").is_none());
    }
}
