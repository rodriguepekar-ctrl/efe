//! M26 — orphan microVM slot reaper.
//!
//! Runs every 5 minutes ; for each per-tenant pool it inspects the
//! `Running`-state slots and verifies that an active `agent_run` row
//! references the slot's `vm_id`. Slots without a backing run AND
//! older than `STALE_THRESHOLD` are killed and removed from the pool.

use crate::sandbox::pool::MicrovmPool;
use crate::{AgentRunnerResult, TenantId};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// A `Running` slot older than this without a backing run is reaped.
pub const STALE_THRESHOLD: Duration = Duration::from_secs(300);
pub const REAP_INTERVAL: Duration = Duration::from_secs(300);

pub async fn reaper_loop(
    pg: PgPool,
    pools: Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>,
) {
    tracing::info!("orphan slot reaper loop started");
    let mut tick = tokio::time::interval(REAP_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if let Err(e) = reap_once(&pg, &pools).await {
            tracing::error!(?e, "reaper iteration failed");
        }
    }
}

async fn reap_once(
    pg: &PgPool,
    pools: &Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>,
) -> AgentRunnerResult<()> {
    let snapshot: Vec<(String, Arc<MicrovmPool>)> = {
        let g = pools.lock().await;
        g.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    for (tenant, pool) in snapshot {
        let vms_in_use: Vec<String> = sqlx::query_scalar(
            r#"SELECT DISTINCT vm_id FROM agent_run
               WHERE tenant_id = $1
                 AND vm_id IS NOT NULL
                 AND status NOT IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')"#,
        )
        .bind(&tenant)
        .fetch_all(pg)
        .await?;
        let in_use: HashSet<String> = vms_in_use.into_iter().collect();

        let stale = pool.list_stale(&in_use, STALE_THRESHOLD).await;
        for vm_id in stale {
            tracing::warn!(
                %tenant,
                vm_id = %vm_id,
                "reaping orphan microVM slot"
            );
            let _ = pool.release(&vm_id).await;
            crate::metrics::AGENT_MICROVM_REAPED_TOTAL
                .with_label_values(&[&TenantId(tenant.clone()).0])
                .inc();
        }
    }
    Ok(())
}
