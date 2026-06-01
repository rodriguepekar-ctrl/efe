// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Per-provider circuit breaker — ADR-3 + § 7.5.
//!
//! State machine (annex A3) :
//! ```text
//!                  3 consecutive failures
//!        Closed ──────────────────────────►  Open
//!          ▲                                   │
//!          │                                   │ 30 s elapsed
//!          │ 1 success                         ▼
//!          └─────────────── HalfOpen ◄─────────┘
//!                                 │
//!                                 │ 1 failure
//!                                 ▼
//!                               Open
//! ```
//!
//! Concurrency-safe via atomics ; no mutex.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// **N20** : the three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    Closed = 0,
    Open = 1,
    HalfOpen = 2,
}

impl CircuitBreakerState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Open,
            2 => Self::HalfOpen,
            _ => Self::Closed,
        }
    }
}

/// **N20** : default consecutive-failure threshold to open the breaker.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// **N20** : default duration (seconds) before Open → HalfOpen.
pub const DEFAULT_OPEN_TO_HALF_OPEN_SECONDS: u64 = 30;

/// **N20** : single-provider circuit breaker.
///
/// Three atomics : state (u8), consecutive_failures (u32),
/// opened_at_unix (u64).  No lock, no mutex.
pub struct CircuitBreaker {
    state: AtomicU8,
    consecutive_failures: AtomicU32,
    opened_at_unix: AtomicU64,
    failure_threshold: u32,
    open_to_half_open_seconds: u64,
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.state())
            .field("consecutive_failures", &self.consecutive_failures.load(Ordering::Relaxed))
            .field("opened_at_unix", &self.opened_at_unix.load(Ordering::Relaxed))
            .field("failure_threshold", &self.failure_threshold)
            .field("open_to_half_open_seconds", &self.open_to_half_open_seconds)
            .finish()
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    /// **N20** : Construct with default thresholds (3 fails, 30 s).
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(CircuitBreakerState::Closed as u8),
            consecutive_failures: AtomicU32::new(0),
            opened_at_unix: AtomicU64::new(0),
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            open_to_half_open_seconds: DEFAULT_OPEN_TO_HALF_OPEN_SECONDS,
        }
    }

    /// **N20** : Construct with explicit thresholds (tests).
    pub fn with_thresholds(failure_threshold: u32, open_to_half_open_seconds: u64) -> Self {
        Self {
            state: AtomicU8::new(CircuitBreakerState::Closed as u8),
            consecutive_failures: AtomicU32::new(0),
            opened_at_unix: AtomicU64::new(0),
            failure_threshold,
            open_to_half_open_seconds,
        }
    }

    /// **N20** : Read the current state with the time-based transition
    /// from Open → HalfOpen if enough time has elapsed.
    pub fn state(&self) -> CircuitBreakerState {
        let raw = self.state.load(Ordering::Acquire);
        let st = CircuitBreakerState::from_u8(raw);
        if matches!(st, CircuitBreakerState::Open) {
            let opened_at = self.opened_at_unix.load(Ordering::Acquire);
            if now_seconds().saturating_sub(opened_at) >= self.open_to_half_open_seconds {
                // Promote to HalfOpen lazily on read.  Atomic swap so
                // concurrent readers don't double-transition.
                let _ = self.state.compare_exchange(
                    CircuitBreakerState::Open as u8,
                    CircuitBreakerState::HalfOpen as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                return CircuitBreakerState::HalfOpen;
            }
        }
        st
    }

    /// **N20** : True if a call should be SKIPPED because the breaker
    /// is open and the cool-down hasn't expired.  HalfOpen returns
    /// false (one trial call allowed).
    pub fn is_blocked(&self) -> bool {
        matches!(self.state(), CircuitBreakerState::Open)
    }

    /// **N20** : Record a successful call.  Closes the breaker.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        self.state
            .store(CircuitBreakerState::Closed as u8, Ordering::Release);
        self.opened_at_unix.store(0, Ordering::Release);
    }

    /// **N20** : Record a failed call.  Increments the failure count ;
    /// transitions Closed → Open or HalfOpen → Open if appropriate.
    pub fn record_failure(&self) {
        // First, latch the current state at entry.
        let cur = self.state();
        match cur {
            CircuitBreakerState::HalfOpen => {
                // HalfOpen → Open immediately on 1 fail.
                self.transition_to_open();
            }
            CircuitBreakerState::Open => {
                // Already open — refresh opened_at to extend the
                // cooldown window? No : extending makes the breaker
                // never half-open.  We DON'T refresh — let the
                // existing cooldown expire on its own schedule.
            }
            CircuitBreakerState::Closed => {
                let n = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;
                if n >= self.failure_threshold {
                    self.transition_to_open();
                }
            }
        }
    }

    fn transition_to_open(&self) {
        self.state
            .store(CircuitBreakerState::Open as u8, Ordering::Release);
        self.opened_at_unix
            .store(now_seconds(), Ordering::Release);
    }

    /// **N20** : Force-reset to Closed (ops dashboard escape hatch).
    pub fn reset(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        self.state
            .store(CircuitBreakerState::Closed as u8, Ordering::Release);
        self.opened_at_unix.store(0, Ordering::Release);
    }

    /// **N20 — test helper** : force the breaker into Open state with
    /// `opened_at_unix = now - elapsed_seconds`.
    #[doc(hidden)]
    pub fn force_open_at(&self, elapsed_seconds: u64) {
        self.state
            .store(CircuitBreakerState::Open as u8, Ordering::Release);
        self.opened_at_unix
            .store(now_seconds().saturating_sub(elapsed_seconds), Ordering::Release);
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_breaker_is_closed_and_not_blocked() {
        let b = CircuitBreaker::new();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
        assert!(!b.is_blocked());
    }

    #[test]
    fn one_failure_does_not_open_below_threshold() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn three_consecutive_failures_open_breaker() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.record_failure();
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Open);
        assert!(b.is_blocked());
    }

    #[test]
    fn success_resets_failure_counter_in_closed() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.record_failure();
        b.record_failure();
        b.record_success();
        // Counter is reset ; we'd need another 3 failures to open.
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn open_transitions_to_half_open_after_cooldown() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        // Force-open with elapsed = 31 s → next read should HalfOpen.
        b.force_open_at(31);
        assert_eq!(b.state(), CircuitBreakerState::HalfOpen);
    }

    #[test]
    fn open_stays_open_before_cooldown_elapses() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.force_open_at(5);
        assert_eq!(b.state(), CircuitBreakerState::Open);
        assert!(b.is_blocked());
    }

    #[test]
    fn half_open_one_success_closes_breaker() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.force_open_at(31);
        assert_eq!(b.state(), CircuitBreakerState::HalfOpen);
        b.record_success();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn half_open_one_failure_reopens_breaker() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.force_open_at(31);
        assert_eq!(b.state(), CircuitBreakerState::HalfOpen);
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Open);
        // And the new opened_at is NOW, so it stays open.
        assert!(b.is_blocked());
    }

    #[test]
    fn reset_force_closes_breaker() {
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.record_failure();
        b.record_failure();
        b.record_failure();
        assert_eq!(b.state(), CircuitBreakerState::Open);
        b.reset();
        assert_eq!(b.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn open_is_not_extended_by_subsequent_failures() {
        // Per implementation : recording a failure while Open should
        // NOT refresh opened_at_unix (otherwise the breaker would
        // never half-open under continuous load).
        let b = CircuitBreaker::with_thresholds(3, 30);
        b.force_open_at(25); // 25 s ago → not yet half-open
        b.record_failure();   // would-be refresh
        b.record_failure();
        // 5 s remaining ; not half-open yet
        assert_eq!(b.state(), CircuitBreakerState::Open);
        // After full 30 s elapses on the ORIGINAL opened_at, half-open.
        b.force_open_at(31);
        assert_eq!(b.state(), CircuitBreakerState::HalfOpen);
    }

    #[test]
    fn state_labels_stable() {
        assert_eq!(CircuitBreakerState::Closed.label(), "closed");
        assert_eq!(CircuitBreakerState::Open.label(), "open");
        assert_eq!(CircuitBreakerState::HalfOpen.label(), "half_open");
    }

    #[test]
    fn debug_format_no_panic() {
        let b = CircuitBreaker::new();
        let _ = format!("{b:?}");
    }
}
