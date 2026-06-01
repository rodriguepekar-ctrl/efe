// SPDX-License-Identifier: UNLICENSED
//! M30 — Cross-region health probing.
//!
//! Periodically probes each peer region's `/v1/region/health`
//! endpoint and surfaces a `PeerHealth` snapshot. Used by :
//!
//! - the failover orchestrator to decide whether a peer is promotable
//! - the DR drill harness to assert pre/post conditions
//! - Prometheus metrics (gauge `efe_region_peer_up{region=…}`)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::current::CurrentRegion;
use crate::error::{RegionError, Result};
use crate::metrics::{REGION_PEER_PROBE_DURATION_MS, REGION_PEER_UP};

/// JSON payload returned by `/v1/region/health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerHealth {
    /// Peer region name.
    pub region: String,
    /// `true` iff the peer answered 200 and reports `ready=true`.
    pub up: bool,
    /// Probe round-trip in milliseconds.
    pub rtt_ms: u64,
    /// Replication lag in seconds (Postgres logical replication slot).
    /// `None` if the peer didn't report it.
    pub replication_lag_secs: Option<f64>,
    /// `true` iff the peer is `passive` AND ready to be promoted.
    pub ready_for_promote: bool,
}

/// Probe configuration.
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    /// HTTP request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Connect timeout in milliseconds.
    pub connect_ms: u64,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 1_500,
            connect_ms: 500,
        }
    }
}

/// Async health probe.
#[derive(Debug, Clone)]
pub struct HealthProber {
    inner: Client,
    cfg: ProbeConfig,
}

impl HealthProber {
    /// Build a new probe with a fresh reqwest client.
    pub fn new(cfg: ProbeConfig) -> Result<Self> {
        let inner = Client::builder()
            .user_agent("efe-region/0.0.1")
            .connect_timeout(Duration::from_millis(cfg.connect_ms))
            .build()
            .map_err(|e| RegionError::Internal {
                reason: format!("reqwest build: {e}"),
            })?;
        Ok(Self { inner, cfg })
    }

    /// Probe one peer. Updates `REGION_PEER_UP` and
    /// `REGION_PEER_PROBE_DURATION_MS`.
    pub async fn probe(&self, region: &str, base_url: &str) -> Result<PeerHealth> {
        let url = format!("{}/v1/region/health", base_url.trim_end_matches('/'));
        let started = Instant::now();
        let resp = tokio::time::timeout(
            Duration::from_millis(self.cfg.timeout_ms),
            self.inner.get(&url).send(),
        )
        .await;

        let elapsed = started.elapsed().as_millis() as u64;
        REGION_PEER_PROBE_DURATION_MS
            .with_label_values(&[region])
            .observe(elapsed as f64);

        match resp {
            Ok(Ok(r)) => {
                if !r.status().is_success() {
                    REGION_PEER_UP.with_label_values(&[region]).set(0);
                    return Err(RegionError::PeerUnhealthy {
                        region: region.into(),
                        reason: format!("non-2xx status {}", r.status().as_u16()),
                    });
                }
                let mut body: PeerHealth = r.json().await.map_err(|e| RegionError::PeerUnhealthy {
                    region: region.into(),
                    reason: format!("body decode: {e}"),
                })?;
                body.rtt_ms = elapsed;
                REGION_PEER_UP
                    .with_label_values(&[region])
                    .set(if body.up { 1 } else { 0 });
                Ok(body)
            }
            Ok(Err(e)) => {
                REGION_PEER_UP.with_label_values(&[region]).set(0);
                warn!(region = %region, error = %e, "peer probe transport error");
                Err(RegionError::ProbeTransport { source: e })
            }
            Err(_elapsed) => {
                REGION_PEER_UP.with_label_values(&[region]).set(0);
                Err(RegionError::ProbeTimeout {
                    elapsed_ms: self.cfg.timeout_ms,
                })
            }
        }
    }

    /// Probe every peer of the given `CurrentRegion`. Returns a map
    /// from region name to either the health snapshot or the
    /// probe error.
    pub async fn probe_all(
        &self,
        current: &CurrentRegion,
    ) -> HashMap<String, Result<PeerHealth>> {
        let mut out = HashMap::new();
        for (region, url) in &current.peer_base_urls {
            let r = self.probe(region, url).await;
            out.insert(region.clone(), r);
        }
        out
    }
}

/// Long-running supervisor that probes every peer every `tick_ms`.
/// Used by `efe-server` startup hook.
pub async fn run_periodic_supervisor(
    prober: HealthProber,
    current: Arc<CurrentRegion>,
    tick_ms: u64,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(tick_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let _ = prober.probe_all(current.as_ref()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_config_defaults_sane() {
        let cfg = ProbeConfig::default();
        assert!(cfg.timeout_ms >= 500);
        assert!(cfg.connect_ms <= cfg.timeout_ms);
    }

    #[test]
    fn health_prober_builds() {
        let _ = HealthProber::new(ProbeConfig::default()).unwrap();
    }

    #[tokio::test]
    async fn probe_unreachable_returns_transport_or_timeout() {
        let prober = HealthProber::new(ProbeConfig {
            timeout_ms: 200,
            connect_ms: 100,
        })
        .unwrap();
        let err = prober.probe("test", "http://127.0.0.1:1").await.unwrap_err();
        assert!(matches!(
            err,
            RegionError::ProbeTransport { .. } | RegionError::ProbeTimeout { .. }
        ));
    }
}
