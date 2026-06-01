// SPDX-License-Identifier: UNLICENSED
//! M30 — Postgres replication-lag monitor (data-source-agnostic).
//!
//! M30 audit fix — the `efe_region_replication_lag_secs` gauge was
//! registered but had no writer ; the `M30RegionHighReplicationLag`
//! alert was therefore dead code. This module supplies the writer
//! loop, abstracted over the actual lag-fetching mechanism so the
//! crate stays free of `sqlx`.
//!
//! ## Wiring
//!
//! `efe-server` provides a concrete `LagFetcher` that executes :
//!
//! ```sql
//! SELECT slot_name,
//!        EXTRACT(epoch FROM now() - last_msg_send_time) AS lag_secs
//!   FROM pg_replication_slots
//!  WHERE slot_type = 'logical' AND active = true;
//! ```
//!
//! and returns the result as a `Vec<(String, f64)>`. The loop below
//! polls this fetcher every `interval` and updates the gauge.
//!
//! On the **passive** region the equivalent query is :
//!
//! ```sql
//! SELECT 'apply_lag' AS slot_name,
//!        EXTRACT(epoch FROM now() - pg_last_xact_replay_timestamp()) AS lag_secs;
//! ```
//!
//! Both flavors can be plugged into the same loop.

use std::time::Duration;

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::Result;
use crate::metrics::set_replication_lag_secs;

/// Per-slot replication lag in seconds.
#[derive(Debug, Clone)]
pub struct SlotLag {
    /// Logical replication slot name (or a synthetic name like
    /// `apply_lag` for the passive-side query).
    pub slot_name: String,
    /// Lag in seconds (may be negative briefly if the clock is
    /// skewed ; the gauge setter clamps to 0).
    pub lag_secs: f64,
}

/// Source of lag information. Implemented by `efe-server` against a
/// `PgPool`.
#[async_trait]
pub trait LagFetcher: Send + Sync {
    /// Fetch the current lag for every active slot.
    async fn fetch(&self) -> Result<Vec<SlotLag>>;
}

/// Run the replication-lag monitor forever. Designed to be spawned as
/// a tokio task at process startup :
///
/// ```ignore
/// let fetcher = PgLagFetcher::new(pool);
/// tokio::spawn(run_lag_monitor(Arc::new(fetcher), Duration::from_secs(15)));
/// ```
///
/// The loop never returns ; a single `fetcher.fetch()` error is
/// logged and the loop continues.
pub async fn run_lag_monitor(
    fetcher: std::sync::Arc<dyn LagFetcher>,
    interval: Duration,
) -> ! {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match fetcher.fetch().await {
            Ok(slots) => {
                for s in slots {
                    set_replication_lag_secs(&s.slot_name, s.lag_secs);
                    debug!(slot = %s.slot_name, lag_secs = s.lag_secs, "replication lag observed");
                }
            }
            Err(e) => {
                warn!(error = %e, "replication-lag fetch failed; gauges left stale");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RegionError;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    struct FakeFetcher {
        calls: AtomicU64,
    }

    #[async_trait]
    impl LagFetcher for FakeFetcher {
        async fn fetch(&self) -> Result<Vec<SlotLag>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(vec![SlotLag {
                    slot_name: "slot_fake_1".into(),
                    lag_secs: 7.4,
                }])
            } else {
                Err(RegionError::Internal {
                    reason: "transient PG error".into(),
                })
            }
        }
    }

    #[tokio::test]
    async fn monitor_updates_gauge_and_survives_errors() {
        let fetcher = Arc::new(FakeFetcher {
            calls: AtomicU64::new(0),
        });
        // Run for ~ 250 ms (≥ 2 ticks at 100 ms) then abort.
        let task = tokio::spawn(run_lag_monitor(fetcher.clone(), Duration::from_millis(100)));
        tokio::time::sleep(Duration::from_millis(250)).await;
        task.abort();

        // First tick wrote the gauge ; second tick errored but did
        // not panic — gauge still holds the prior value.
        let observed = crate::metrics::REGION_REPLICATION_LAG_SECS
            .with_label_values(&["slot_fake_1"])
            .get();
        assert_eq!(observed, 7); // 7.4 rounds to 7
        assert!(fetcher.calls.load(Ordering::SeqCst) >= 2);
    }
}
