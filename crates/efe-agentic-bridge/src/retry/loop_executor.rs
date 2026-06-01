// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : retry loop driver.
//! See architecture § 9.2 (state machine) and § 19.2 (skeleton).

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_nano_sdk::RetryOutcome;

use crate::retry::{RetryDecision, RetryOutcomeProbe, RetryPolicy};

/// **N27** : outcome of a single attempt.
///
/// **Audit-fix Bug #2** : the `Failure` variant additionally carries
/// an opaque `partial: Option<P>` so the caller can preserve per-attempt
/// state (e.g. the `StreamState` with its `abort_reason` populated)
/// across the retry boundary.  The retry loop returns ONLY the LAST
/// attempt's partial state — see `RetryLoop::execute`.
pub enum AttemptOutcome<T, E, P = ()> {
    Success(T),
    Failure {
        error: E,
        probe: RetryOutcomeProbe,
        partial: Option<P>,
    },
}

impl<T, E, P> AttemptOutcome<T, E, P> {
    /// **N27** : convenience constructor for a failure with no
    /// partial state (`partial = None`).
    pub fn fail(error: E, probe: RetryOutcomeProbe) -> Self {
        Self::Failure {
            error,
            probe,
            partial: None,
        }
    }

    /// **N27** : convenience constructor for a failure with a
    /// partial state attached.
    pub fn fail_with(error: E, probe: RetryOutcomeProbe, partial: P) -> Self {
        Self::Failure {
            error,
            probe,
            partial: Some(partial),
        }
    }
}

/// **N27** : final tally stamped onto the N17 record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrySummary {
    pub retry_count: u8,
    pub outcome: RetryOutcome,
    pub total_wait_ms: u32,
}

impl RetrySummary {
    /// **N27** : the "no retry happened" sentinel.
    pub fn no_retry() -> Self {
        Self {
            retry_count: 0,
            outcome: RetryOutcome::NoRetry,
            total_wait_ms: 0,
        }
    }
}

/// **N27 + § 19.2** : the retry loop executor.
///
/// Wraps an async `attempt_fn` and drives it through the
/// [`RetryPolicy`].  Returns the eventual `Result<T, E>` along
/// with a [`RetrySummary`] that the bridge stamps onto the
/// emitted N17 record.
///
/// The executor is generic over the attempt function's output
/// and error types so it can be reused for non-streaming flows
/// `(where T is a full response)` and streaming flows
/// `(where T is a StreamState from a single completed turn)`.
pub struct RetryLoop<P: RetryPolicy> {
    /// The classification policy.
    pub policy: P,
    /// **ADR-N27-3** : shared flag flipped to `true` the moment
    /// the bridge sends its first downstream event.  After that
    /// the loop refuses to retry — every classification gets the
    /// probe with `post_first_byte_downstream = true`.
    pub downstream_sent_first_event: Arc<AtomicBool>,
    /// Total wall-clock cap on retry sleeps.  Falls back to
    /// `policy.max_total_wall` when unset.
    pub max_total_wall: Duration,
}

impl<P: RetryPolicy> RetryLoop<P> {
    /// **N27** : convenience constructor with default cap.
    pub fn new(policy: P) -> Self {
        Self {
            policy,
            downstream_sent_first_event: Arc::new(AtomicBool::new(false)),
            max_total_wall: Duration::from_millis(super::backoff::N27_DEFAULT_MAX_TOTAL_WALL_MS),
        }
    }

    /// **N27** : build with an externally-controlled "first byte"
    /// flag — used by the streaming dispatch path where the flag
    /// is also read by the SSE encoder.
    pub fn with_first_byte_flag(policy: P, flag: Arc<AtomicBool>) -> Self {
        Self {
            policy,
            downstream_sent_first_event: flag,
            max_total_wall: Duration::from_millis(super::backoff::N27_DEFAULT_MAX_TOTAL_WALL_MS),
        }
    }

    /// **N27** : execute the loop.
    ///
    /// `attempt_fn` is called once per attempt, given the
    /// 0-indexed attempt number.  It returns an
    /// [`AttemptOutcome<T, E, P>`].
    ///
    /// On success : returns `(Ok(t), None_partial, RetrySummary)`.
    /// On exhaustion : returns `(Err(final_error), Some_partial_or_None, RetrySummary)`.
    /// The partial state is the LAST attempt's `partial` (audit-fix
    /// Bug #2 — abort_reason preservation across retry boundary).
    pub async fn execute<F, T, E, PartialState, Fut>(
        &self,
        mut attempt_fn: F,
    ) -> (Result<T, E>, Option<PartialState>, RetrySummary)
    where
        F: FnMut(u8) -> Fut,
        Fut: Future<Output = AttemptOutcome<T, E, PartialState>>,
    {
        let mut attempt: u8 = 0;
        let mut total_wait = Duration::ZERO;
        let _started = Instant::now();
        loop {
            let outcome = attempt_fn(attempt).await;
            match outcome {
                AttemptOutcome::Success(t) => {
                    let label = if attempt == 0 {
                        RetryOutcome::NoRetry
                    } else {
                        RetryOutcome::RetriedSucceeded
                    };
                    return (
                        Ok(t),
                        None,
                        RetrySummary {
                            retry_count: attempt,
                            outcome: label,
                            total_wait_ms: total_wait.as_millis() as u32,
                        },
                    );
                }
                AttemptOutcome::Failure {
                    error,
                    mut probe,
                    partial,
                } => {
                    // Inject the live post-byte flag (the
                    // attempt fn may not know about it).
                    probe.post_first_byte_downstream |=
                        self.downstream_sent_first_event.load(Ordering::SeqCst);

                    match self.policy.classify(attempt, &probe) {
                        RetryDecision::Retry { delay } => {
                            // Total wall guard.
                            if total_wait + delay > self.max_total_wall {
                                return (
                                    Err(error),
                                    partial,
                                    RetrySummary {
                                        retry_count: attempt,
                                        outcome: RetryOutcome::RetriedExhausted,
                                        total_wait_ms: total_wait.as_millis() as u32,
                                    },
                                );
                            }
                            tracing::warn!(
                                attempt,
                                next_delay_ms = delay.as_millis() as u64,
                                http_status = probe.http_status,
                                network_class = probe.network_class.label(),
                                post_first_byte_downstream = probe.post_first_byte_downstream,
                                "N27 retry scheduled"
                            );
                            tokio::time::sleep(delay).await;
                            total_wait += delay;
                            attempt = attempt.saturating_add(1);
                            if attempt > self.policy.max_retries() {
                                return (
                                    Err(error),
                                    partial,
                                    RetrySummary {
                                        retry_count: attempt.saturating_sub(1),
                                        outcome: RetryOutcome::RetriedExhausted,
                                        total_wait_ms: total_wait.as_millis() as u32,
                                    },
                                );
                            }
                        }
                        RetryDecision::Abort => {
                            let label = if attempt == 0 {
                                RetryOutcome::NonRetryable
                            } else {
                                RetryOutcome::RetriedExhausted
                            };
                            return (
                                Err(error),
                                partial,
                                RetrySummary {
                                    retry_count: attempt,
                                    outcome: label,
                                    total_wait_ms: total_wait.as_millis() as u32,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::ExponentialBackoff;

    #[tokio::test]
    async fn success_first_try_returns_no_retry() {
        let loop_ = RetryLoop::new(ExponentialBackoff::default());
        let (res, partial, summary) = loop_
            .execute(|_| async {
                AttemptOutcome::<i32, &'static str, ()>::Success(42)
            })
            .await;
        assert_eq!(res, Ok(42));
        assert!(partial.is_none());
        assert_eq!(summary.retry_count, 0);
        assert_eq!(summary.outcome, RetryOutcome::NoRetry);
        assert_eq!(summary.total_wait_ms, 0);
    }

    #[tokio::test]
    async fn one_fail_then_success_records_retried_succeeded() {
        let policy = ExponentialBackoff {
            jitter_pct: 0.0,
            ..Default::default()
        };
        let loop_ = RetryLoop::new(policy);
        let attempts_seen = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let attempts_clone = attempts_seen.clone();
        let (res, partial, summary) = loop_
            .execute(move |attempt| {
                let attempts = attempts_clone.clone();
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        AttemptOutcome::<i32, String, ()>::fail(
                            "503".into(),
                            RetryOutcomeProbe::http(503),
                        )
                    } else {
                        AttemptOutcome::Success(7)
                    }
                }
            })
            .await;
        assert_eq!(res, Ok(7));
        assert!(partial.is_none());
        assert_eq!(summary.retry_count, 1);
        assert_eq!(summary.outcome, RetryOutcome::RetriedSucceeded);
        assert!(summary.total_wait_ms >= 450 && summary.total_wait_ms <= 550);
        assert_eq!(attempts_seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn always_fails_records_retried_exhausted() {
        let policy = ExponentialBackoff {
            jitter_pct: 0.0,
            base: Duration::from_millis(10),
            max_per_retry: Duration::from_millis(20),
            ..Default::default()
        };
        let loop_ = RetryLoop::new(policy);
        let attempts_seen = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let attempts_clone = attempts_seen.clone();
        let (res, _partial, summary) = loop_
            .execute(move |_| {
                let attempts = attempts_clone.clone();
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    AttemptOutcome::<i32, String, ()>::fail(
                        "503".into(),
                        RetryOutcomeProbe::http(503),
                    )
                }
            })
            .await;
        assert!(res.is_err());
        assert_eq!(attempts_seen.load(Ordering::SeqCst), 4);
        assert_eq!(summary.outcome, RetryOutcome::RetriedExhausted);
    }

    #[tokio::test]
    async fn non_retryable_400_first_try_records_non_retryable() {
        let loop_ = RetryLoop::new(ExponentialBackoff::default());
        let (res, _partial, summary) = loop_
            .execute(|_| async {
                AttemptOutcome::<i32, String, ()>::fail(
                    "400".into(),
                    RetryOutcomeProbe::http(400),
                )
            })
            .await;
        assert!(res.is_err());
        assert_eq!(summary.retry_count, 0);
        assert_eq!(summary.outcome, RetryOutcome::NonRetryable);
        assert_eq!(summary.total_wait_ms, 0);
    }

    #[tokio::test]
    async fn first_byte_downstream_flag_forces_abort() {
        let policy = ExponentialBackoff::default();
        let flag = Arc::new(AtomicBool::new(true));
        let loop_ = RetryLoop::with_first_byte_flag(policy, flag);
        let (res, _partial, summary) = loop_
            .execute(|_| async {
                AttemptOutcome::<i32, String, ()>::fail(
                    "503".into(),
                    RetryOutcomeProbe::http(503),
                )
            })
            .await;
        assert!(res.is_err());
        assert_eq!(summary.retry_count, 0);
        assert_eq!(summary.outcome, RetryOutcome::NonRetryable);
    }

    #[tokio::test]
    async fn total_wall_cap_exits_early() {
        let policy = ExponentialBackoff {
            jitter_pct: 0.0,
            base: Duration::from_millis(50),
            max_per_retry: Duration::from_millis(50),
            max_retries: 10,
            ..Default::default()
        };
        let mut loop_ = RetryLoop::new(policy);
        loop_.max_total_wall = Duration::from_millis(75);
        let (res, _partial, summary) = loop_
            .execute(|_| async {
                AttemptOutcome::<i32, String, ()>::fail(
                    "503".into(),
                    RetryOutcomeProbe::http(503),
                )
            })
            .await;
        assert!(res.is_err());
        assert_eq!(summary.outcome, RetryOutcome::RetriedExhausted);
        assert!(summary.total_wait_ms <= 75);
    }

    // ─── Audit-fix Bug #2 : partial state preserved across retries ─

    #[tokio::test]
    async fn partial_state_preserved_through_exhaustion() {
        let policy = ExponentialBackoff {
            jitter_pct: 0.0,
            base: Duration::from_millis(5),
            max_per_retry: Duration::from_millis(10),
            ..Default::default()
        };
        let loop_ = RetryLoop::new(policy);
        let attempt_seen = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let attempt_clone = attempt_seen.clone();
        let (res, partial, summary) = loop_
            .execute(move |attempt| {
                let counter = attempt_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    // The "partial state" is the attempt index.
                    AttemptOutcome::<i32, String, u8>::fail_with(
                        format!("503 attempt {attempt}"),
                        RetryOutcomeProbe::http(503),
                        attempt,
                    )
                }
            })
            .await;
        assert!(res.is_err());
        // The partial from the LAST attempt is returned.
        assert_eq!(partial, Some(3));
        assert_eq!(summary.outcome, RetryOutcome::RetriedExhausted);
        assert_eq!(attempt_seen.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn partial_state_preserved_on_non_retryable() {
        let loop_ = RetryLoop::new(ExponentialBackoff::default());
        let (res, partial, summary) = loop_
            .execute(|_| async {
                AttemptOutcome::<i32, String, &'static str>::fail_with(
                    "400".into(),
                    RetryOutcomeProbe::http(400),
                    "abort_reason_marker",
                )
            })
            .await;
        assert!(res.is_err());
        assert_eq!(partial, Some("abort_reason_marker"));
        assert_eq!(summary.outcome, RetryOutcome::NonRetryable);
    }
}
