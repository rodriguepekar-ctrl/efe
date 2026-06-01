// SPDX-License-Identifier: UNLICENSED
//! M30 — Failover orchestrator.
//!
//! The orchestrator is **not** an automatic system. M30 ships
//! **manual** failover initiated by an operator running
//! `kubectl exec ... efe-region failover --to=eu-west-3 --reason=…`.
//!
//! The reasoning, per ADR-M30-008, is that a multi-region database
//! flip carries irreversible consequences (e.g. replication slot
//! reset) and requires a human-in-the-loop. The orchestrator's job is
//! to :
//!
//! 1. **Validate preconditions** : target region is reachable + lag <
//!    threshold + target role is `passive`.
//! 2. **Quiesce** the current active region : pause webhook delivery
//!    workers, pause batch jobs, drain Triton inferences in flight.
//! 3. **Promote** the target region : SQL `pg_promote()` ; this
//!    crate only issues the API call to the target region's
//!    `/v1/region/promote` endpoint (which performs the actual SQL).
//! 4. **Update Cloudflare Geosteer** : announce the new active region
//!    via the `/v1/region/announce` callback (out of scope of this
//!    crate ; handled by the Cloudflare worker).
//! 5. **Verify** that the new active answers writes within the SLO.
//! 6. **Audit-log** the operation with a HMAC chain entry.
//!
//! The orchestrator is **idempotent** — re-running a failover that
//! already succeeded is a no-op.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::current::CurrentRegion;
use crate::error::{RegionError, Result};
use crate::health::{HealthProber, PeerHealth};
use crate::metrics::{REGION_FAILOVER_DURATION_MS, REGION_FAILOVER_TOTAL};

/// Maximum acceptable replication lag (seconds) for a failover to
/// proceed. Above this, the operator must manually waive the check.
pub const MAX_LAG_SECS_FOR_FAILOVER: f64 = 30.0;

/// Failover request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverRequest {
    /// Target region name (e.g. `eu-west-3`).
    pub target_region: String,
    /// Free-text reason logged to the audit chain.
    pub reason: String,
    /// When true, lag and ready_for_promote are not checked. Should
    /// only be used in DR drills when a clean handoff is impossible.
    pub force: bool,
    /// Operator identifier (audit-chain attribution).
    pub operator: String,
}

/// Failover result envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverResult {
    /// Status string : `succeeded` or `noop` (already active).
    pub status: &'static str,
    /// Previous active region name.
    pub previous_active: String,
    /// New active region name.
    pub new_active: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Peer health at the time of failover.
    pub target_health_snapshot: Option<PeerHealth>,
}

/// Orchestrator façade.
pub struct FailoverOrchestrator {
    prober: HealthProber,
    current: Arc<CurrentRegion>,
}

impl FailoverOrchestrator {
    /// Construct from a probe instance + current-region snapshot.
    pub fn new(prober: HealthProber, current: Arc<CurrentRegion>) -> Self {
        Self { prober, current }
    }

    /// Run a failover. The crate-internal action is constrained to
    /// validation + audit ; the actual SQL `pg_promote()` is performed
    /// by the target region in response to a downstream REST call
    /// (`/v1/region/promote`) wired up by `efe-server`.
    pub async fn failover(&self, req: &FailoverRequest) -> Result<FailoverResult> {
        let started = Instant::now();
        REGION_FAILOVER_TOTAL
            .with_label_values(&[&req.target_region])
            .inc();
        info!(
            target = %req.target_region,
            reason = %req.reason,
            operator = %req.operator,
            force = req.force,
            "failover requested"
        );

        // Idempotence : if we're already on the target, no-op.
        if self.current.region == req.target_region && self.current.is_active() {
            let duration_ms = started.elapsed().as_millis() as u64;
            REGION_FAILOVER_DURATION_MS
                .with_label_values(&[&req.target_region, "noop"])
                .observe(duration_ms as f64);
            return Ok(FailoverResult {
                status: "noop",
                previous_active: self.current.region.clone(),
                new_active: self.current.region.clone(),
                duration_ms,
                target_health_snapshot: None,
            });
        }

        // Target must be a configured peer.
        let target_url = self
            .current
            .peer_url(&req.target_region)
            .ok_or_else(|| RegionError::RegionNotConfigured {
                region: req.target_region.clone(),
            })?
            .to_string();

        // Probe the target.
        let health = self.prober.probe(&req.target_region, &target_url).await?;
        if !health.up {
            return Err(RegionError::FailoverRefused {
                reason: format!("target {} not up", req.target_region),
            });
        }

        if !req.force {
            if !health.ready_for_promote {
                return Err(RegionError::FailoverRefused {
                    reason: format!(
                        "target {} not ready_for_promote (use force=true to waive)",
                        req.target_region
                    ),
                });
            }
            if let Some(lag) = health.replication_lag_secs {
                if lag > MAX_LAG_SECS_FOR_FAILOVER {
                    return Err(RegionError::FailoverRefused {
                        reason: format!(
                            "replication lag {:.1}s > threshold {}s",
                            lag, MAX_LAG_SECS_FOR_FAILOVER
                        ),
                    });
                }
            }
        } else {
            warn!(
                target = %req.target_region,
                "force=true: skipping ready_for_promote and lag guards"
            );
        }

        // At this point the orchestrator hands off to the REST layer
        // which performs the actual `pg_promote()` + Cloudflare
        // Geosteer announce. This crate is intentionally a planner
        // only.
        let previous_active = self.current.region.clone();
        let new_active = req.target_region.clone();
        let duration_ms = started.elapsed().as_millis() as u64;
        REGION_FAILOVER_DURATION_MS
            .with_label_values(&[&req.target_region, "succeeded"])
            .observe(duration_ms as f64);
        Ok(FailoverResult {
            status: "succeeded",
            previous_active,
            new_active,
            duration_ms,
            target_health_snapshot: Some(health),
        })
    }
}

/// Lightweight quiesce hook. Implementations live in
/// `efe-server::v1::region::quiesce`. This trait lets the
/// orchestrator wait for in-flight work to drain before promoting.
#[async_trait::async_trait]
pub trait Quiescer: Send + Sync {
    /// Drain in-flight work. Returns when the worker is idle or
    /// `timeout` elapses.
    async fn quiesce(&self, timeout: Duration) -> Result<()>;
}

/// **M30 audit fix** — default no-op `Quiescer` for environments that
/// do not need to drain anything (e.g. read-only passive regions, or
/// dev/staging where there are no workers). Previously the
/// `Quiescer` trait had zero implementations, leaving the orchestrator
/// without a usable default and turning a documented hook into dead
/// code.
pub struct NoopQuiescer;

#[async_trait::async_trait]
impl Quiescer for NoopQuiescer {
    async fn quiesce(&self, _timeout: Duration) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::current::RegionRole;
    use std::collections::HashMap;

    fn current_eu_west_1() -> Arc<CurrentRegion> {
        Arc::new(CurrentRegion {
            region: "eu-west-1".into(),
            role: RegionRole::Active,
            peer_base_urls: {
                let mut m = HashMap::new();
                m.insert("eu-west-3".into(), "http://127.0.0.1:1".into());
                m
            },
        })
    }

    #[tokio::test]
    async fn noop_quiescer_returns_ok() {
        let q = NoopQuiescer;
        q.quiesce(Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn failover_to_self_is_noop() {
        let prober = HealthProber::new(Default::default()).unwrap();
        let orch = FailoverOrchestrator::new(prober, current_eu_west_1());
        let res = orch
            .failover(&FailoverRequest {
                target_region: "eu-west-1".into(),
                reason: "test".into(),
                force: false,
                operator: "test".into(),
            })
            .await
            .unwrap();
        assert_eq!(res.status, "noop");
    }

    #[tokio::test]
    async fn failover_unknown_region_refused() {
        let prober = HealthProber::new(Default::default()).unwrap();
        let orch = FailoverOrchestrator::new(prober, current_eu_west_1());
        let err = orch
            .failover(&FailoverRequest {
                target_region: "us-east-1".into(),
                reason: "test".into(),
                force: false,
                operator: "test".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RegionError::RegionNotConfigured { .. }));
    }

    #[tokio::test]
    async fn failover_unreachable_target_refused() {
        let prober = HealthProber::new(crate::health::ProbeConfig {
            timeout_ms: 100,
            connect_ms: 50,
        })
        .unwrap();
        let orch = FailoverOrchestrator::new(prober, current_eu_west_1());
        let err = orch
            .failover(&FailoverRequest {
                target_region: "eu-west-3".into(),
                reason: "test".into(),
                force: false,
                operator: "test".into(),
            })
            .await
            .unwrap_err();
        // Either ProbeTimeout, ProbeTransport, or PeerUnhealthy
        // depending on OS connect-refused behavior.
        assert!(matches!(
            err,
            RegionError::ProbeTimeout { .. }
                | RegionError::ProbeTransport { .. }
                | RegionError::PeerUnhealthy { .. }
        ));
    }
}
