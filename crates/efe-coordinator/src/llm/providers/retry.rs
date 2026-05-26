// SPDX-License-Identifier: UNLICENSED
//! `RetryPolicy` — exponential backoff avec jitter.
//!
//! Algorithme : `delay = base × 2^attempt + jitter` (capped à `max_delay`).
//! Jitter pseudo-aléatoire 0..25% du delay pour éviter les avalanches.

use std::future::Future;
use std::time::Duration;

use crate::error::{CoordError, CoordResult};

/// Politique de retry par défaut Phase 3 R1.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Délai initial (1ère tentative de retry).
    pub base_delay: Duration,
    /// Délai max (cap exponential).
    pub max_delay: Duration,
    /// Nombre max de retries (en plus de la tentative initiale).
    pub max_retries: u32,
    /// Pour deterministic testing : si Some, utilise cette valeur au lieu du jitter aléatoire.
    pub deterministic_jitter_ms: Option<u64>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(16),
            max_retries: 3,
            deterministic_jitter_ms: None,
        }
    }
}

impl RetryPolicy {
    /// Politique no-retry (utilisé pour les erreurs non-recoverable).
    #[must_use]
    pub fn none() -> Self {
        Self {
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            max_retries: 0,
            deterministic_jitter_ms: None,
        }
    }

    /// Calcule le délai pour la tentative `attempt` (0-indexed).
    ///
    /// Formule : `base × 2^attempt + jitter`, capped à `max_delay`.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let exp = 1_u64.checked_shl(attempt.min(10)).unwrap_or(1024);
        let base_ms = self.base_delay.as_millis() as u64;
        let with_exp = base_ms.saturating_mul(exp);
        let jitter = self.deterministic_jitter_ms.unwrap_or_else(|| {
            // Pseudo-aléatoire bon marché à partir du temps (pas de dep `rand`).
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64)
                .unwrap_or(0);
            nanos % (with_exp / 4).max(1)
        });
        let total_ms = (with_exp + jitter).min(self.max_delay.as_millis() as u64);
        Duration::from_millis(total_ms)
    }
}

/// Détermine si une erreur justifie un retry (politique Phase 3 R1).
///
/// Note : la décision finale est dans le provider lui-même (qui voit
/// les codes HTTP). Cette helper centralise la logique.
#[must_use]
pub fn should_retry(err: &CoordError) -> bool {
    match err {
        CoordError::LlmProvider { reason, .. } => {
            // Heuristique sur le message — les vrais providers exposent leurs
            // codes HTTP via le `reason` (préfixé "HTTP 429" / "HTTP 5xx" / "transport").
            reason.contains("HTTP 429")
                || reason.contains("HTTP 500")
                || reason.contains("HTTP 502")
                || reason.contains("HTTP 503")
                || reason.contains("HTTP 504")
                || reason.contains("HTTP 529") // Anthropic overload
                || reason.contains("transport")
                || reason.contains("timeout")
        }
        _ => false,
    }
}

/// Exécute `f` avec retry selon `policy`.
///
/// Sémantique : `f` est appelé au moins une fois ; en cas d'erreur
/// retryable, on attend le `delay_for_attempt(n)` puis on rappelle.
/// Échec définitif après `max_retries` retries.
pub async fn retry_with_backoff<F, Fut, T>(
    policy: &RetryPolicy,
    mut f: F,
) -> CoordResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = CoordResult<T>>,
{
    let mut last_err: Option<CoordError> = None;
    for attempt in 0..=policy.max_retries {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !should_retry(&e) || attempt == policy.max_retries {
                    return Err(e);
                }
                let delay = policy.delay_for_attempt(attempt);
                tokio::time::sleep(delay).await;
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| CoordError::Config("retry loop exited unexpectedly".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn default_policy_has_3_retries() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_retries, 3);
        assert_eq!(p.base_delay, Duration::from_millis(500));
        assert_eq!(p.max_delay, Duration::from_secs(16));
    }

    #[test]
    fn delay_increases_exponentially() {
        let p = RetryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            max_retries: 5,
            deterministic_jitter_ms: Some(0),
        };
        assert_eq!(p.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(p.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(p.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(p.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn delay_capped_at_max() {
        let p = RetryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            max_retries: 10,
            deterministic_jitter_ms: Some(0),
        };
        // 100 × 2^3 = 800 → cap à 500
        assert_eq!(p.delay_for_attempt(3), Duration::from_millis(500));
        assert_eq!(p.delay_for_attempt(10), Duration::from_millis(500));
    }

    #[test]
    fn should_retry_429() {
        let e = CoordError::LlmProvider {
            provider: "claude".into(),
            reason: "HTTP 429: rate limit".into(),
        };
        assert!(should_retry(&e));
    }

    #[test]
    fn should_retry_5xx() {
        for code in ["500", "502", "503", "504", "529"] {
            let e = CoordError::LlmProvider {
                provider: "x".into(),
                reason: format!("HTTP {code}: error"),
            };
            assert!(should_retry(&e), "should retry on HTTP {code}");
        }
    }

    #[test]
    fn should_not_retry_4xx_except_429() {
        for code in ["400", "401", "403", "404"] {
            let e = CoordError::LlmProvider {
                provider: "x".into(),
                reason: format!("HTTP {code}: client error"),
            };
            assert!(!should_retry(&e), "should NOT retry on HTTP {code}");
        }
    }

    #[test]
    fn should_retry_transport() {
        let e = CoordError::LlmProvider {
            provider: "x".into(),
            reason: "transport: connection refused".into(),
        };
        assert!(should_retry(&e));
    }

    #[tokio::test]
    async fn retry_succeeds_after_2_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let p = RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            max_retries: 3,
            deterministic_jitter_ms: Some(0),
        };
        let c = counter.clone();
        let result: CoordResult<u32> = retry_with_backoff(&p, move || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 3 {
                    Err(CoordError::LlmProvider {
                        provider: "test".into(),
                        reason: "HTTP 503: try again".into(),
                    })
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_fails_after_max_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let p = RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            max_retries: 2,
            deterministic_jitter_ms: Some(0),
        };
        let c = counter.clone();
        let result: CoordResult<()> = retry_with_backoff(&p, move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(CoordError::LlmProvider {
                    provider: "test".into(),
                    reason: "HTTP 503: always fail".into(),
                })
            }
        })
        .await;
        assert!(result.is_err());
        // 1 initial + 2 retries = 3 attempts.
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn no_retry_on_non_retryable() {
        let counter = Arc::new(AtomicU32::new(0));
        let p = RetryPolicy::default();
        let c = counter.clone();
        let result: CoordResult<()> = retry_with_backoff(&p, move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err(CoordError::LlmProvider {
                    provider: "test".into(),
                    reason: "HTTP 401: unauthorized".into(),
                })
            }
        })
        .await;
        assert!(result.is_err());
        // 1 attempt seulement, pas de retry.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
