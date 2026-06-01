// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : exponential backoff with jitter +
//! `Retry-After` parser.  See architecture § 4.2 and § 4.4.

use std::time::{Duration, SystemTime};

use crate::retry::classify::classify_default;
use crate::retry::{RetryDecision, RetryOutcomeProbe, RetryPolicy};

// ─────────────────────────────────────────────────────────────────────
//  Defaults  (architecture § 4.2 + § 17.1 env-var matrix)
// ─────────────────────────────────────────────────────────────────────

/// **N27** : default base delay for the exponential curve.
pub const N27_DEFAULT_BASE_MS: u64 = 500;
/// **N27** : default per-retry delay cap.
pub const N27_DEFAULT_MAX_PER_RETRY_MS: u64 = 4_000;
/// **N27** : default total wall-clock retry budget.
pub const N27_DEFAULT_MAX_TOTAL_WALL_MS: u64 = 30_000;
/// **N27 + I22** : default max retries (hard bound).
pub const N27_DEFAULT_MAX_RETRIES: u8 = 3;
/// **N27** : jitter ratio (±25 %).
pub const N27_DEFAULT_JITTER_PCT: f64 = 0.25;
/// **N27** : 529 "Overloaded" specific base (Anthropic recommends
/// at least 1 s).
pub const N27_DEFAULT_OVERLOADED_BASE_MS: u64 = 1_000;
/// **N27** : 529 specific cap.
pub const N27_DEFAULT_OVERLOADED_MAX_MS: u64 = 8_000;
/// **N27 + T7** : cap on parsed `Retry-After` (anti header-forgery
/// via proxy : 5 min).
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

// ─────────────────────────────────────────────────────────────────────
//  ExponentialBackoff
// ─────────────────────────────────────────────────────────────────────

/// **N27** : the default retry policy — exponential backoff
/// with jitter, honours `Retry-After`, classifies HTTP / network
/// status per architecture § 4.1 table.
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    pub base: Duration,
    pub multiplier: f64,
    pub max_per_retry: Duration,
    pub max_total_wall: Duration,
    pub max_retries: u8,
    pub jitter_pct: f64,
    pub overloaded_base: Duration,
    pub overloaded_max: Duration,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(N27_DEFAULT_BASE_MS),
            multiplier: 2.0,
            max_per_retry: Duration::from_millis(N27_DEFAULT_MAX_PER_RETRY_MS),
            max_total_wall: Duration::from_millis(N27_DEFAULT_MAX_TOTAL_WALL_MS),
            max_retries: N27_DEFAULT_MAX_RETRIES,
            jitter_pct: N27_DEFAULT_JITTER_PCT,
            overloaded_base: Duration::from_millis(N27_DEFAULT_OVERLOADED_BASE_MS),
            overloaded_max: Duration::from_millis(N27_DEFAULT_OVERLOADED_MAX_MS),
        }
    }
}

impl ExponentialBackoff {
    /// **N27** : build from env vars (see architecture § 17.1).
    /// Missing or malformed values fall back to the constant
    /// defaults — never panic.
    pub fn from_env() -> Self {
        fn env_u64(k: &str, default: u64) -> u64 {
            std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
        }
        fn env_u8(k: &str, default: u8) -> u8 {
            std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
        }
        fn env_f64(k: &str, default: f64) -> f64 {
            std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
        }
        Self {
            base: Duration::from_millis(env_u64("EFE_BRIDGE_RETRY_BASE_MS", N27_DEFAULT_BASE_MS)),
            multiplier: 2.0,
            max_per_retry: Duration::from_millis(env_u64(
                "EFE_BRIDGE_RETRY_MAX_PER_MS",
                N27_DEFAULT_MAX_PER_RETRY_MS,
            )),
            max_total_wall: Duration::from_millis(env_u64(
                "EFE_BRIDGE_RETRY_MAX_TOTAL_MS",
                N27_DEFAULT_MAX_TOTAL_WALL_MS,
            )),
            max_retries: env_u8("EFE_BRIDGE_RETRY_MAX", N27_DEFAULT_MAX_RETRIES),
            jitter_pct: env_f64("EFE_BRIDGE_RETRY_JITTER_PCT", N27_DEFAULT_JITTER_PCT),
            overloaded_base: Duration::from_millis(env_u64(
                "EFE_BRIDGE_RETRY_OVERLOADED_BASE_MS",
                N27_DEFAULT_OVERLOADED_BASE_MS,
            )),
            overloaded_max: Duration::from_millis(env_u64(
                "EFE_BRIDGE_RETRY_OVERLOADED_MAX_MS",
                N27_DEFAULT_OVERLOADED_MAX_MS,
            )),
        }
    }

    /// **N27 + ADR-N27-9** : a no-retry policy (max_retries=0).
    /// Used by tests and by callers that opt out via
    /// `EFE_BRIDGE_RETRY_ENABLED=0`.
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// **N27** : compute the sleep duration for the `attempt`-th
    /// retry (0-indexed : `attempt=0` is the FIRST retry after
    /// the initial failure).
    ///
    /// Honours `Retry-After` first ; otherwise falls back to the
    /// exponential curve, with a 529-specific branch.
    ///
    /// **Note** : jitter is NOT applied here — the caller is
    /// expected to wrap with [`Self::apply_jitter`] if it wants
    /// randomization.  Splitting these two operations lets unit
    /// tests pin determinism while still exercising the formula.
    pub fn compute_delay(&self, attempt: u8, probe: &RetryOutcomeProbe) -> Duration {
        // 1. Retry-After honored exactly (capped at MAX_RETRY_AFTER).
        if let Some(ra) = probe.retry_after {
            return ra.min(MAX_RETRY_AFTER);
        }

        // 2. Overloaded (529) uses its own base.
        let is_overloaded = probe.http_status == Some(529)
            || probe.anthropic_error_type.as_deref() == Some("overloaded_error");
        let (base, cap) = if is_overloaded {
            (self.overloaded_base, self.overloaded_max)
        } else {
            (self.base, self.max_per_retry)
        };

        // 3. Exponential : base * multiplier^attempt
        let factor = self.multiplier.powi(i32::from(attempt));
        let raw_ms = (base.as_millis() as f64 * factor).min(cap.as_millis() as f64);
        Duration::from_millis(raw_ms as u64)
    }

    /// **N27** : apply ±`jitter_pct` jitter to a duration.
    /// Uses [`rand::thread_rng`] under the hood.
    pub fn apply_jitter(&self, d: Duration) -> Duration {
        if self.jitter_pct <= 0.0 {
            return d;
        }
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let factor: f64 = rng.gen_range(-self.jitter_pct..=self.jitter_pct);
        let adjusted = d.as_millis() as f64 * (1.0 + factor);
        let clamped = adjusted.max(0.0);
        Duration::from_millis(clamped as u64)
    }
}

impl RetryPolicy for ExponentialBackoff {
    fn classify(&self, attempt: u8, probe: &RetryOutcomeProbe) -> RetryDecision {
        // **ADR-N27-3** : never retry after first byte downstream.
        if probe.post_first_byte_downstream {
            return RetryDecision::Abort;
        }
        // **I22** : honour max_retries (the loop also enforces).
        if attempt >= self.max_retries {
            return RetryDecision::Abort;
        }
        // Defer to the classifier table.
        match classify_default(probe) {
            RetryDecision::Retry { .. } => {
                let delay = self.apply_jitter(self.compute_delay(attempt, probe));
                RetryDecision::Retry { delay }
            }
            RetryDecision::Abort => RetryDecision::Abort,
        }
    }

    fn max_retries(&self) -> u8 {
        self.max_retries
    }
}

// ─────────────────────────────────────────────────────────────────────
//  parse_retry_after  (RFC 7231 § 7.1.3)
// ─────────────────────────────────────────────────────────────────────

/// **N27 + § 4.4** : parse a `Retry-After` header value.
///
/// Accepts either delta-seconds (integer) or HTTP-date.  Caps
/// the result at [`MAX_RETRY_AFTER`] (anti header-forgery via
/// proxy, threat T7).  Returns `None` on malformed input.
pub fn parse_retry_after(header_value: &str) -> Option<Duration> {
    let trimmed = header_value.trim();
    // Delta-seconds.
    if let Ok(n) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(n).min(MAX_RETRY_AFTER));
    }
    // HTTP-date.
    if let Ok(when) = httpdate::parse_http_date(trimmed) {
        let now = SystemTime::now();
        return when
            .duration_since(now)
            .ok()
            .map(|d| d.min(MAX_RETRY_AFTER));
    }
    None
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_architecture_section_4_2() {
        let p = ExponentialBackoff::default();
        assert_eq!(p.base, Duration::from_millis(500));
        assert!((p.multiplier - 2.0).abs() < f64::EPSILON);
        assert_eq!(p.max_per_retry, Duration::from_secs(4));
        assert_eq!(p.max_total_wall, Duration::from_secs(30));
        assert_eq!(p.max_retries, 3);
        assert!((p.jitter_pct - 0.25).abs() < f64::EPSILON);
        assert_eq!(p.overloaded_base, Duration::from_secs(1));
        assert_eq!(p.overloaded_max, Duration::from_secs(8));
    }

    #[test]
    fn compute_delay_exponential_curve_no_jitter() {
        let p = ExponentialBackoff::default();
        let probe = RetryOutcomeProbe::http(503);
        // attempt 0 → base * 2^0 = 500ms
        assert_eq!(p.compute_delay(0, &probe), Duration::from_millis(500));
        // attempt 1 → base * 2^1 = 1000ms
        assert_eq!(p.compute_delay(1, &probe), Duration::from_millis(1_000));
        // attempt 2 → base * 2^2 = 2000ms
        assert_eq!(p.compute_delay(2, &probe), Duration::from_millis(2_000));
        // attempt 3 → base * 2^3 = 4000ms (== cap)
        assert_eq!(p.compute_delay(3, &probe), Duration::from_millis(4_000));
        // attempt 4 → base * 2^4 = 8000ms → cap 4000ms
        assert_eq!(p.compute_delay(4, &probe), Duration::from_millis(4_000));
    }

    #[test]
    fn compute_delay_529_uses_overloaded_base() {
        let p = ExponentialBackoff::default();
        let probe = RetryOutcomeProbe::http(529);
        assert_eq!(p.compute_delay(0, &probe), Duration::from_millis(1_000));
        assert_eq!(p.compute_delay(1, &probe), Duration::from_millis(2_000));
        assert_eq!(p.compute_delay(2, &probe), Duration::from_millis(4_000));
        assert_eq!(p.compute_delay(3, &probe), Duration::from_millis(8_000));
        // capped at overloaded_max.
        assert_eq!(p.compute_delay(4, &probe), Duration::from_millis(8_000));
    }

    #[test]
    fn compute_delay_anthropic_overloaded_error_type_uses_overloaded_curve() {
        let p = ExponentialBackoff::default();
        let mut probe = RetryOutcomeProbe::http(500);
        probe.anthropic_error_type = Some("overloaded_error".into());
        // First retry uses overloaded_base = 1000 ms.
        assert_eq!(p.compute_delay(0, &probe), Duration::from_millis(1_000));
    }

    #[test]
    fn compute_delay_honors_retry_after_exactly() {
        let p = ExponentialBackoff::default();
        let mut probe = RetryOutcomeProbe::http(429);
        probe.retry_after = Some(Duration::from_secs(7));
        // Retry-After overrides curve.
        assert_eq!(p.compute_delay(0, &probe), Duration::from_secs(7));
        // Even on attempt 3 (where curve would be 4 s).
        assert_eq!(p.compute_delay(3, &probe), Duration::from_secs(7));
    }

    #[test]
    fn compute_delay_caps_retry_after_at_5_min() {
        let p = ExponentialBackoff::default();
        let mut probe = RetryOutcomeProbe::http(429);
        probe.retry_after = Some(Duration::from_secs(999_999));
        assert_eq!(p.compute_delay(0, &probe), MAX_RETRY_AFTER);
    }

    #[test]
    fn classify_post_first_byte_always_aborts() {
        let p = ExponentialBackoff::default();
        let mut probe = RetryOutcomeProbe::http(503);
        probe.post_first_byte_downstream = true;
        assert_eq!(p.classify(0, &probe), RetryDecision::Abort);
    }

    #[test]
    fn classify_attempt_at_or_past_max_aborts() {
        let p = ExponentialBackoff::default();
        let probe = RetryOutcomeProbe::http(503);
        assert!(matches!(
            p.classify(2, &probe),
            RetryDecision::Retry { .. }
        ));
        // attempt == max_retries → abort.
        assert_eq!(p.classify(3, &probe), RetryDecision::Abort);
        assert_eq!(p.classify(4, &probe), RetryDecision::Abort);
    }

    #[test]
    fn classify_400_does_not_retry() {
        let p = ExponentialBackoff::default();
        let probe = RetryOutcomeProbe::http(400);
        assert_eq!(p.classify(0, &probe), RetryDecision::Abort);
    }

    #[test]
    fn classify_503_retries_with_curve_delay() {
        let p = ExponentialBackoff::default();
        let probe = RetryOutcomeProbe::http(503);
        // Use jitter=0 variant for determinism.
        let p_no_jitter = ExponentialBackoff {
            jitter_pct: 0.0,
            ..p
        };
        assert_eq!(
            p_no_jitter.classify(0, &probe),
            RetryDecision::Retry {
                delay: Duration::from_millis(500),
            }
        );
    }

    #[test]
    fn apply_jitter_zero_pct_is_identity() {
        let p = ExponentialBackoff {
            jitter_pct: 0.0,
            ..Default::default()
        };
        let d = Duration::from_millis(1_000);
        assert_eq!(p.apply_jitter(d), d);
    }

    #[test]
    fn apply_jitter_within_pct_band() {
        let p = ExponentialBackoff::default();
        let d = Duration::from_millis(1_000);
        for _ in 0..1_000 {
            let j = p.apply_jitter(d);
            let low = (1.0 - p.jitter_pct) * 1_000.0;
            let high = (1.0 + p.jitter_pct) * 1_000.0;
            assert!(
                j.as_millis() as f64 >= low - 1.0,
                "jitter {}ms < {}ms",
                j.as_millis(),
                low
            );
            assert!(
                j.as_millis() as f64 <= high + 1.0,
                "jitter {}ms > {}ms",
                j.as_millis(),
                high
            );
        }
    }

    // ─── Retry-After parser ───────────────────────────────────────

    #[test]
    fn parse_retry_after_delta_seconds() {
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
    }

    #[test]
    fn parse_retry_after_caps_at_5_minutes() {
        assert_eq!(parse_retry_after("9999999"), Some(MAX_RETRY_AFTER));
    }

    #[test]
    fn parse_retry_after_http_date_in_future() {
        // 1 minute from now → ~60 s.
        let future = SystemTime::now() + Duration::from_secs(60);
        let s = httpdate::fmt_http_date(future);
        let d = parse_retry_after(&s).unwrap();
        // tolerate ±2 s of clock skew between fmt + parse.
        assert!(
            d >= Duration::from_secs(58) && d <= Duration::from_secs(62),
            "got {d:?}"
        );
    }

    #[test]
    fn parse_retry_after_http_date_in_past_returns_none() {
        // Past date → duration_since returns Err → None.
        let past = SystemTime::now() - Duration::from_secs(60);
        let s = httpdate::fmt_http_date(past);
        assert!(parse_retry_after(&s).is_none());
    }

    #[test]
    fn parse_retry_after_garbage_returns_none() {
        assert!(parse_retry_after("not a date or number").is_none());
        assert!(parse_retry_after("").is_none());
        assert!(parse_retry_after("-5").is_none());
    }

    #[test]
    fn disabled_policy_max_retries_zero() {
        let p = ExponentialBackoff::disabled();
        assert_eq!(p.max_retries, 0);
        let probe = RetryOutcomeProbe::http(503);
        // attempt 0 already at the cap → abort.
        assert_eq!(p.classify(0, &probe), RetryDecision::Abort);
    }
}
