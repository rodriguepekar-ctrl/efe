//! Circuit breaker for the Presidio backend.
//!
//! State machine :
//!
//! ```text
//!   Closed ── 3 fails in a row ──▶ Open
//!     ▲                              │
//!     │                              │ after `open_duration`
//!     │                              ▼
//!   any success ◀── 1 fail ──── HalfOpen
//! ```
//!
//! While Open, every call returns `Err(CircuitOpen)` immediately
//! without contacting the backend.

use std::sync::atomic::{AtomicI64, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Circuit state encoded as `u8` for atomic ops.
const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

/// Trip threshold : consecutive failures before opening.
pub const DEFAULT_TRIP_AT_FAILS: u32 = 3;

/// Open duration before transitioning to half-open.
pub const DEFAULT_OPEN_DURATION: Duration = Duration::from_secs(30);

/// Lock-free circuit breaker shared across async tasks.
#[derive(Clone, Debug)]
pub struct CircuitBreaker {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: AtomicU8,
    /// Stored as u64 to avoid platform-specific AtomicU32 availability ;
    /// the actual count never exceeds the trip threshold (~10) so the
    /// upper 32 bits are always zero.
    consecutive_fails: AtomicU64,
    open_until_unix_ns: AtomicI64,
    open_duration: Duration,
    trip_at_fails: u32,
}

impl CircuitBreaker {
    /// Build with default thresholds.
    pub fn new() -> Self {
        Self::with_config(DEFAULT_TRIP_AT_FAILS, DEFAULT_OPEN_DURATION)
    }

    /// Build with custom thresholds.
    pub fn with_config(trip_at_fails: u32, open_duration: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: AtomicU8::new(STATE_CLOSED),
                consecutive_fails: AtomicU64::new(0),
                open_until_unix_ns: AtomicI64::new(0),
                open_duration,
                trip_at_fails,
            }),
        }
    }

    /// Check whether the breaker permits a call right now. When it
    /// returns false, the caller should short-circuit with
    /// `Unavailable("circuit open")`.
    pub fn allow(&self) -> bool {
        let st = self.inner.state.load(Ordering::Acquire);
        match st {
            STATE_CLOSED => true,
            STATE_OPEN => {
                // Check if the open duration has elapsed.
                let now = now_unix_ns();
                let until = self.inner.open_until_unix_ns.load(Ordering::Acquire);
                if now >= until {
                    // Transition Open → HalfOpen.
                    self.inner
                        .state
                        .store(STATE_HALF_OPEN, Ordering::Release);
                    true
                } else {
                    false
                }
            }
            STATE_HALF_OPEN => true,
            _ => true, // unreachable
        }
    }

    /// Record a successful call. Resets the failure counter.
    pub fn record_success(&self) {
        self.inner.consecutive_fails.store(0, Ordering::Release);
        let st = self.inner.state.load(Ordering::Acquire);
        if st == STATE_HALF_OPEN {
            self.inner.state.store(STATE_CLOSED, Ordering::Release);
        }
    }

    /// Record a failed call. May transition Closed → Open or HalfOpen → Open.
    pub fn record_failure(&self) {
        let n = self.inner.consecutive_fails.fetch_add(1, Ordering::AcqRel) + 1;
        let st = self.inner.state.load(Ordering::Acquire);
        match st {
            STATE_CLOSED => {
                if n >= self.inner.trip_at_fails as u64 {
                    self.trip();
                }
            }
            STATE_HALF_OPEN => {
                // A single failure in half-open re-opens.
                self.trip();
            }
            STATE_OPEN => { /* still open ; nothing to do */ }
            _ => {}
        }
    }

    fn trip(&self) {
        let open_until = now_unix_ns()
            .saturating_add(self.inner.open_duration.as_nanos() as i64);
        self.inner
            .open_until_unix_ns
            .store(open_until, Ordering::Release);
        self.inner.state.store(STATE_OPEN, Ordering::Release);
    }

    /// State for telemetry.
    pub fn current_state(&self) -> &'static str {
        match self.inner.state.load(Ordering::Acquire) {
            STATE_CLOSED => "closed",
            STATE_OPEN => "open",
            STATE_HALF_OPEN => "half_open",
            _ => "unknown",
        }
    }

    /// Numeric for Prometheus gauge.
    pub fn current_state_code(&self) -> u8 {
        self.inner.state.load(Ordering::Acquire)
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

fn now_unix_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed_and_allows() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.current_state(), "closed");
        assert!(cb.allow());
    }

    #[test]
    fn trips_after_threshold_fails() {
        let cb = CircuitBreaker::with_config(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.current_state(), "closed");
        cb.record_failure();
        assert_eq!(cb.current_state(), "open");
        assert!(!cb.allow());
    }

    #[test]
    fn success_resets_counter() {
        let cb = CircuitBreaker::with_config(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        cb.record_failure();
        cb.record_failure();
        // Still closed because counter was reset.
        assert_eq!(cb.current_state(), "closed");
    }

    #[test]
    fn half_open_after_duration() {
        let cb = CircuitBreaker::with_config(1, Duration::from_millis(10));
        cb.record_failure();
        assert_eq!(cb.current_state(), "open");
        std::thread::sleep(Duration::from_millis(15));
        // allow() triggers the transition.
        assert!(cb.allow());
        assert_eq!(cb.current_state(), "half_open");
    }

    #[test]
    fn half_open_to_closed_on_success() {
        let cb = CircuitBreaker::with_config(1, Duration::from_millis(10));
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(15));
        assert!(cb.allow()); // → half_open
        cb.record_success();
        assert_eq!(cb.current_state(), "closed");
    }

    #[test]
    fn half_open_to_open_on_fail() {
        let cb = CircuitBreaker::with_config(1, Duration::from_millis(10));
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(15));
        assert!(cb.allow()); // → half_open
        cb.record_failure();
        assert_eq!(cb.current_state(), "open");
    }
}
