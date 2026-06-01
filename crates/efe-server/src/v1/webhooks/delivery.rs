//! Delivery-related types : retry schedule, circuit breaker, outcome.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Backoff schedule for failed deliveries.
/// Index 0 = first retry, ..., index 5 = last (= 6 attempts total
/// counting the initial try).
pub const RETRY_DELAYS_SECONDS: &[u64] = &[
    60,        // 1 min
    5 * 60,    // 5 min
    30 * 60,   // 30 min
    4 * 3600,  // 4 h
    24 * 3600, // 24 h
];

/// Maximum delivery attempts (initial + retries).
pub const MAX_ATTEMPTS: u8 = 1 + RETRY_DELAYS_SECONDS.len() as u8;

/// Per-attempt outcome enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryOutcome {
    /// 2xx received from customer ; no further retry.
    Ok,
    /// 4xx (except 408/425/429) ; permanent failure, do not retry.
    Fail,
    /// 5xx, 408, 425, 429, network error, timeout — schedule retry.
    Retry,
    /// Reached `MAX_ATTEMPTS` ; drop event but keep audit log.
    Drop,
}

impl DeliveryOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
            Self::Retry => "retry",
            Self::Drop => "drop",
        }
    }
}

/// Classify an HTTP status / error into a delivery outcome.
pub fn classify_status(status: Option<u16>) -> DeliveryOutcome {
    match status {
        Some(s) if (200..300).contains(&s) => DeliveryOutcome::Ok,
        Some(408) | Some(425) | Some(429) => DeliveryOutcome::Retry,
        Some(s) if (400..500).contains(&s) => DeliveryOutcome::Fail,
        Some(s) if (500..600).contains(&s) => DeliveryOutcome::Retry,
        Some(_) => DeliveryOutcome::Retry,
        None => DeliveryOutcome::Retry, // network / timeout
    }
}

/// Compute the `next_retry_at` timestamp given current attempt count
/// (1-indexed : `attempt = 1` is the first try). Returns `None` if no
/// more retries are allowed.
pub fn next_retry_at(now: DateTime<Utc>, attempt: u8) -> Option<DateTime<Utc>> {
    if attempt == 0 || attempt as usize > RETRY_DELAYS_SECONDS.len() {
        return None;
    }
    let secs = RETRY_DELAYS_SECONDS[(attempt - 1) as usize];
    Some(now + Duration::seconds(secs as i64))
}

/// Circuit breaker state for a single webhook subscription.
///
/// Transitions :
/// - Closed → Open : 10 consecutive failed deliveries.
/// - Open → HalfOpen : after `open_duration` (default 1 h).
/// - HalfOpen → Closed : 1 successful delivery in half-open state.
/// - HalfOpen → Open : 1 failure resets to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

pub const CIRCUIT_TRIP_AT_FAILS: u32 = 10;
pub const CIRCUIT_OPEN_DURATION_SECONDS: i64 = 3600;

/// Apply a delivery outcome to the breaker state. Returns the new
/// `(state, consecutive_fails, circuit_opens_at)` triple.
pub fn apply_outcome(
    current_state: CircuitState,
    consecutive_fails: u32,
    outcome: DeliveryOutcome,
    now: DateTime<Utc>,
) -> (CircuitState, u32, Option<DateTime<Utc>>) {
    match (current_state, outcome) {
        // ── Closed ──
        (CircuitState::Closed, DeliveryOutcome::Ok)
        | (CircuitState::Closed, DeliveryOutcome::Fail) => (
            CircuitState::Closed, // even permanent fails don't trip (one bad event)
            0,                    // reset counter on Ok or permanent Fail
            None,
        ),
        (CircuitState::Closed, DeliveryOutcome::Retry)
        | (CircuitState::Closed, DeliveryOutcome::Drop) => {
            let n = consecutive_fails.saturating_add(1);
            if n >= CIRCUIT_TRIP_AT_FAILS {
                (
                    CircuitState::Open,
                    n,
                    Some(now + Duration::seconds(CIRCUIT_OPEN_DURATION_SECONDS)),
                )
            } else {
                (CircuitState::Closed, n, None)
            }
        }
        // ── Open ── (no deliveries happen ; this branch is for
        // manual recovery / time-based transition handled elsewhere).
        (CircuitState::Open, _) => (CircuitState::Open, consecutive_fails, None),
        // ── HalfOpen ──
        (CircuitState::HalfOpen, DeliveryOutcome::Ok) => (CircuitState::Closed, 0, None),
        (CircuitState::HalfOpen, _) => (
            CircuitState::Open,
            consecutive_fails.saturating_add(1),
            Some(now + Duration::seconds(CIRCUIT_OPEN_DURATION_SECONDS)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_2xx_is_ok() {
        assert_eq!(classify_status(Some(200)), DeliveryOutcome::Ok);
        assert_eq!(classify_status(Some(204)), DeliveryOutcome::Ok);
    }

    #[test]
    fn classify_408_429_retry() {
        assert_eq!(classify_status(Some(408)), DeliveryOutcome::Retry);
        assert_eq!(classify_status(Some(429)), DeliveryOutcome::Retry);
    }

    #[test]
    fn classify_400_permanent_fail() {
        assert_eq!(classify_status(Some(400)), DeliveryOutcome::Fail);
        assert_eq!(classify_status(Some(404)), DeliveryOutcome::Fail);
    }

    #[test]
    fn classify_5xx_retry() {
        assert_eq!(classify_status(Some(500)), DeliveryOutcome::Retry);
        assert_eq!(classify_status(Some(503)), DeliveryOutcome::Retry);
    }

    #[test]
    fn classify_network_retry() {
        assert_eq!(classify_status(None), DeliveryOutcome::Retry);
    }

    #[test]
    fn retry_schedule_monotonic() {
        let now = Utc::now();
        let mut last = now;
        for i in 1..=5 {
            let nxt = next_retry_at(now, i).expect("retry");
            assert!(nxt >= last);
            last = nxt;
        }
        assert!(next_retry_at(now, 6).is_none()); // no 6th retry
    }

    #[test]
    fn circuit_trips_after_10_retries() {
        let now = Utc::now();
        let mut state = CircuitState::Closed;
        let mut fails: u32 = 0;
        for _ in 0..9 {
            let (s, f, _) = apply_outcome(state, fails, DeliveryOutcome::Retry, now);
            state = s;
            fails = f;
            assert_eq!(state, CircuitState::Closed);
        }
        let (s, f, opens) = apply_outcome(state, fails, DeliveryOutcome::Retry, now);
        assert_eq!(s, CircuitState::Open);
        assert_eq!(f, 10);
        assert!(opens.is_some());
    }

    #[test]
    fn ok_resets_counter() {
        let now = Utc::now();
        let (s, f, _) = apply_outcome(CircuitState::Closed, 5, DeliveryOutcome::Ok, now);
        assert_eq!(s, CircuitState::Closed);
        assert_eq!(f, 0);
    }

    #[test]
    fn half_open_to_closed_on_ok() {
        let now = Utc::now();
        let (s, f, _) = apply_outcome(CircuitState::HalfOpen, 10, DeliveryOutcome::Ok, now);
        assert_eq!(s, CircuitState::Closed);
        assert_eq!(f, 0);
    }

    #[test]
    fn half_open_to_open_on_fail() {
        let now = Utc::now();
        let (s, _, opens) = apply_outcome(CircuitState::HalfOpen, 10, DeliveryOutcome::Retry, now);
        assert_eq!(s, CircuitState::Open);
        assert!(opens.is_some());
    }
}
