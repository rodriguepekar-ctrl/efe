//! Per-tenant per-endpoint quota counters backed by Redis.
//!
//! Two windows :
//! - **minute** : sliding 60s counter used for rate-limit decisions
//!   (classify, embed, search, agent_run).
//! - **day** : 86400s counter used for daily totals (LLM tokens).
//!
//! Counter semantics : atomic INCR + EXPIRE (only on first write of the
//! window) via a Lua script. Cluster-safe because the script touches a
//! single key.
//!
//! Failure semantics : if Redis is unreachable, callers should **fail
//! open** (let the request through) — the same policy used by Envoy
//! `failure_mode_deny: false`. The local backpressure middleware acts
//! as the safety net.

use crate::client::CacheClient;
use crate::error::CacheResult;
use crate::key;
use crate::metrics;
use tracing::warn;

/// Outcome of a quota check : was the request inside the budget ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCheck {
    /// Allowed ; current usage `current` ≤ `limit`.
    Allow { current: u64, limit: u64 },
    /// Rejected ; current usage > limit. `retry_after_seconds` hints
    /// when the window resets.
    Reject {
        current: u64,
        limit: u64,
        retry_after_seconds: u64,
    },
    /// Backend unavailable ; default to allow (fail-open).
    Unknown,
}

impl QuotaCheck {
    /// Returns true if the request should be allowed through.
    pub fn allowed(&self) -> bool {
        matches!(self, QuotaCheck::Allow { .. } | QuotaCheck::Unknown)
    }
}

/// Per-tenant per-endpoint quota counters.
#[derive(Clone, Debug)]
pub struct QuotaCounters {
    client: CacheClient,
}

impl QuotaCounters {
    pub fn new(client: CacheClient) -> Self {
        Self { client }
    }

    /// Increment the minute-bucket counter for (tenant, endpoint), check
    /// against `limit_per_minute`. Atomic.
    pub async fn check_per_minute(
        &self,
        tenant_slug: &str,
        endpoint: &str,
        limit_per_minute: u64,
    ) -> CacheResult<QuotaCheck> {
        let bucket = key::epoch_minute_now();
        let k = key::quota_key(tenant_slug, endpoint, bucket);
        // 90 s TTL : 60 s window + 30 s grace for clock skew.
        match self.client.incr_with_window(&k, 90).await {
            Ok(v) => {
                metrics::QUOTA_CURRENT
                    .with_label_values(&[tenant_slug, endpoint, "minute"])
                    .set(v as i64);
                if v > limit_per_minute {
                    metrics::QUOTA_EXCEEDED_TOTAL
                        .with_label_values(&[tenant_slug, endpoint])
                        .inc();
                    let now = chrono::Utc::now().timestamp() as u64;
                    let next_min = (bucket + 1) * 60;
                    let retry = next_min.saturating_sub(now).max(1);
                    Ok(QuotaCheck::Reject {
                        current: v,
                        limit: limit_per_minute,
                        retry_after_seconds: retry,
                    })
                } else {
                    Ok(QuotaCheck::Allow {
                        current: v,
                        limit: limit_per_minute,
                    })
                }
            }
            Err(e) => {
                warn!(?e, tenant=%tenant_slug, endpoint=%endpoint, "quota incr failed, fail-open");
                Ok(QuotaCheck::Unknown)
            }
        }
    }

    /// Daily counter for LLM tokens. Use `add` (≥ 1) per call ; the
    /// internal command remains a single INCRBY for atomicity.
    pub async fn check_per_day(
        &self,
        tenant_slug: &str,
        endpoint: &str,
        limit_per_day: u64,
    ) -> CacheResult<QuotaCheck> {
        let bucket = key::epoch_day_now();
        let k = key::quota_key(tenant_slug, endpoint, bucket);
        // 25 h TTL = 86400 + 3600 grace.
        match self.client.incr_with_window(&k, 90_000).await {
            Ok(v) => {
                metrics::QUOTA_CURRENT
                    .with_label_values(&[tenant_slug, endpoint, "day"])
                    .set(v as i64);
                if v > limit_per_day {
                    metrics::QUOTA_EXCEEDED_TOTAL
                        .with_label_values(&[tenant_slug, endpoint])
                        .inc();
                    let now = chrono::Utc::now().timestamp() as u64;
                    let next_day = (bucket + 1) * 86_400;
                    let retry = next_day.saturating_sub(now).max(60);
                    Ok(QuotaCheck::Reject {
                        current: v,
                        limit: limit_per_day,
                        retry_after_seconds: retry,
                    })
                } else {
                    Ok(QuotaCheck::Allow {
                        current: v,
                        limit: limit_per_day,
                    })
                }
            }
            Err(e) => {
                warn!(?e, tenant=%tenant_slug, endpoint=%endpoint, "daily quota incr failed, fail-open");
                Ok(QuotaCheck::Unknown)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_check_allow_logic() {
        let c = QuotaCheck::Allow { current: 5, limit: 10 };
        assert!(c.allowed());
    }

    #[test]
    fn quota_check_reject_logic() {
        let c = QuotaCheck::Reject {
            current: 11,
            limit: 10,
            retry_after_seconds: 30,
        };
        assert!(!c.allowed());
    }

    #[test]
    fn quota_check_unknown_fails_open() {
        // Fail-open : Unknown allows the request through.
        let c = QuotaCheck::Unknown;
        assert!(c.allowed());
    }
}
