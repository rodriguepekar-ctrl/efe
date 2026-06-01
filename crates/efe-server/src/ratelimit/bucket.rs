//! `TokenBucket` — single bucket with refill rate.
//!
//! Non-blocking: callers invoke [`TokenBucket::try_acquire`] which
//! either succeeds immediately or returns the estimated milliseconds
//! until enough tokens are available for the requested cost.

use std::sync::Mutex;
use std::time::Instant;

/// Returned when the bucket does not have enough tokens.
#[derive(Copy, Clone, Debug, thiserror::Error)]
#[error("rate limited: retry after {retry_after_ms} ms")]
pub struct TryAcquireError {
    /// Estimated ms until the missing tokens are refilled.
    pub retry_after_ms: u64,
}

/// Fixed-capacity token bucket with continuous refill.
#[derive(Debug)]
pub struct TokenBucket {
    capacity: f32,
    refill_rate_per_sec: f32,
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    tokens: f32,
    last_refill: Instant,
}

impl TokenBucket {
    /// Build a bucket. Starts full (`tokens = capacity`).
    #[must_use]
    pub fn new(capacity: u32, refill_rate_per_sec: f32) -> Self {
        Self {
            capacity: capacity as f32,
            refill_rate_per_sec,
            state: Mutex::new(BucketState {
                tokens: capacity as f32,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Attempt to consume `n` tokens.
    ///
    /// # Errors
    ///
    /// [`TryAcquireError`] with `retry_after_ms` when not enough tokens.
    pub fn try_acquire(&self, n: u32) -> Result<(), TryAcquireError> {
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.refill(&mut state);
        let cost = n as f32;
        if state.tokens >= cost {
            state.tokens -= cost;
            Ok(())
        } else {
            let missing = cost - state.tokens;
            // Avoid division by zero for a frozen bucket.
            let retry_after_ms = if self.refill_rate_per_sec > 0.0 {
                (missing / self.refill_rate_per_sec * 1000.0).ceil() as u64
            } else {
                u64::MAX
            };
            Err(TryAcquireError { retry_after_ms })
        }
    }

    fn refill(&self, state: &mut BucketState) {
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(state.last_refill)
            .as_secs_f32();
        let added = elapsed * self.refill_rate_per_sec;
        state.tokens = (state.tokens + added).min(self.capacity);
        state.last_refill = now;
    }

    /// Remaining tokens (approximate ; picks up after a refill).
    #[must_use]
    pub fn snapshot_tokens(&self) -> f32 {
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.refill(&mut state);
        state.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn full_bucket_accepts_up_to_capacity() {
        let b = TokenBucket::new(3, 0.0);
        b.try_acquire(1).unwrap();
        b.try_acquire(1).unwrap();
        b.try_acquire(1).unwrap();
        assert!(b.try_acquire(1).is_err());
    }

    #[test]
    fn refill_returns_tokens_after_wait() {
        // 100 tokens/sec → 10ms = 1 token.
        let b = TokenBucket::new(1, 100.0);
        b.try_acquire(1).unwrap();
        assert!(b.try_acquire(1).is_err());
        sleep(Duration::from_millis(20));
        b.try_acquire(1).unwrap();
    }

    #[test]
    fn refill_clamped_to_capacity() {
        let b = TokenBucket::new(5, 10_000.0);
        b.try_acquire(5).unwrap();
        sleep(Duration::from_millis(1));
        assert!(b.snapshot_tokens() <= 5.0 + f32::EPSILON);
    }

    #[test]
    fn retry_after_ms_is_positive_when_rate_limited() {
        let b = TokenBucket::new(1, 10.0);
        b.try_acquire(1).unwrap();
        let err = b.try_acquire(1).unwrap_err();
        assert!(err.retry_after_ms > 0);
        assert!(err.retry_after_ms < 1_000); // reasonable upper bound
    }

    #[test]
    fn zero_refill_rate_never_replenishes() {
        let b = TokenBucket::new(1, 0.0);
        b.try_acquire(1).unwrap();
        sleep(Duration::from_millis(5));
        assert!(b.try_acquire(1).is_err());
    }

    #[test]
    fn try_acquire_n_consumes_n() {
        let b = TokenBucket::new(5, 0.0);
        b.try_acquire(3).unwrap();
        assert!(b.try_acquire(3).is_err());
        b.try_acquire(2).unwrap();
    }

    #[test]
    fn concurrent_acquires_do_not_double_spend() {
        use std::sync::Arc;
        use std::thread;
        let b = Arc::new(TokenBucket::new(100, 0.0));
        let mut handles = Vec::new();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..16 {
            let b = b.clone();
            let count = count.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    if b.try_acquire(1).is_ok() {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // No more than capacity tokens were ever issued.
        assert!(count.load(std::sync::atomic::Ordering::SeqCst) <= 100);
    }
}
