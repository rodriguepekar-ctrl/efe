//! Kubernetes controller — reconciles `AgentRun` CRs with the database
//! state machine.
//!
//! Two reconcile sources :
//! 1. **Postgres** (`agent_run` rows in PENDING / PROVISIONING /
//!    AWAITING_APPROVAL → drive forward).
//! 2. **Kubernetes** (`AgentRun` CR creation / deletion → mirror to
//!    DB and back).
//!
//! Per pilot scale (≤ 3 tenants × 5 concurrent runs = 15 runs total),
//! a 5 s polling loop is more than enough ; we'd switch to watch
//! streams (kube::runtime::watcher) at scale.

use crate::audit::{self, AgentAuditEvent};
use crate::approval;
use crate::run::{self, AgentRunStatus};
use crate::sandbox::{
    apply_network_policy, FirecrackerDriver, MicrovmPool, NetworkPolicySpec, PoolConfig,
};
use crate::sandbox::firecracker::FirecrackerUdsDriver;
use crate::{AgentRunnerResult, TenantId};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Shared per-tenant microVM pool map. The controller and the reaper
/// share the same Arc<Mutex<...>> so the reaper observes the live
/// pools, not an empty side-map.
pub type PoolMap = Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>;

/// D93 — accept a pre-built pool map so `main.rs` can share it with
/// the reaper goroutine. Existing `run(pg, k8s)` is preserved as a
/// thin wrapper for backward compatibility.
pub async fn run_with_pools(
    pg: PgPool,
    k8s: kube::Client,
    pools: PoolMap,
) -> AgentRunnerResult<()> {
    tracing::info!("agent runtime controller starting");
    let driver: Arc<dyn FirecrackerDriver> = Arc::new(FirecrackerUdsDriver::from_env()?);

    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Approval expiry sweeper — every 60 s.
    let pg2 = pg.clone();
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(60));
        loop {
            t.tick().await;
            match approval::sweep_expired(&pg2).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(expired = n, "approval requests expired"),
                Err(e) => tracing::warn!(?e, "approval expiry sweep failed"),
            }
        }
    });

    loop {
        tick.tick().await;
        if let Err(e) = reconcile_once(&pg, &k8s, &pools, &driver).await {
            tracing::error!(?e, "reconcile error");
        }
    }
}

/// Legacy entrypoint. Constructs its own pool map ; prefer
/// `run_with_pools` so the reaper can share the live map.
pub async fn run(pg: PgPool, k8s: kube::Client) -> AgentRunnerResult<()> {
    let pools: PoolMap = Arc::new(Mutex::new(HashMap::new()));
    run_with_pools(pg, k8s, pools).await
}

async fn reconcile_once(
    pg: &PgPool,
    k8s: &kube::Client,
    pools: &Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>,
    driver: &Arc<dyn FirecrackerDriver>,
) -> AgentRunnerResult<()> {
    let pending: Vec<(Uuid, String)> = sqlx::query_as(
        r#"SELECT id, tenant_id FROM agent_run
           WHERE status = 'PENDING'::agent_run_status
           ORDER BY created_at ASC LIMIT 10"#,
    )
    .fetch_all(pg)
    .await?;

    for (run_id, tenant) in pending {
        let tenant = TenantId(tenant);
        if let Err(e) = drive_pending_to_running(pg, k8s, pools, driver, run_id, &tenant).await {
            tracing::error!(?e, %run_id, "failed to drive run to RUNNING");
            // Mark FAILED so the run is not retried forever.
            let mut tx = pg.begin().await?;
            let _ = run::update_status(&mut tx, run_id, AgentRunStatus::Failed).await;
            let _ = audit::append_agent_audit(
                &mut tx, &tenant, Some(run_id), None,
                AgentAuditEvent::RunFailed,
                "system:controller", Some("system"),
                serde_json::json!({"error": e.to_string()}),
            ).await;
            tx.commit().await?;
            crate::metrics::AGENT_FAILURE_RATE_TOTAL
                .with_label_values(&[&tenant.0, "provisioning_failed"])
                .inc();
        }
    }

    Ok(())
}

async fn drive_pending_to_running(
    pg: &PgPool,
    k8s: &kube::Client,
    pools: &Arc<Mutex<HashMap<String, Arc<MicrovmPool>>>>,
    driver: &Arc<dyn FirecrackerDriver>,
    run_id: Uuid,
    tenant: &TenantId,
) -> AgentRunnerResult<()> {
    // 1. PENDING → PROVISIONING (status only ; audit fires when the
    //    microVM is actually up).
    {
        let mut tx = pg.begin().await?;
        run::update_status(&mut tx, run_id, AgentRunStatus::Provisioning).await?;
        tx.commit().await?;
    }

    // 2. Get/create the per-tenant pool.
    let pool = {
        let mut g = pools.lock().await;
        if let Some(p) = g.get(&tenant.0) {
            p.clone()
        } else {
            let p = Arc::new(MicrovmPool::new(
                tenant.clone(),
                PoolConfig::default(),
                driver.clone(),
            ));
            g.insert(tenant.0.clone(), p.clone());
            // Spawn the warm-keeper loop for this tenant.
            tokio::spawn(p.clone().warm_loop());
            p
        }
    };

    // 3. Lease a slot — this actually spawns / reuses a microVM.
    let slot = pool.lease().await?;

    // 4. Stamp vm_id on the run row + audit MicrovmProvisioned.
    {
        let mut tx = pg.begin().await?;
        sqlx::query(r#"UPDATE agent_run SET vm_id = $1 WHERE id = $2"#)
            .bind(&slot.handle.vm_id)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        audit::append_agent_audit(
            &mut tx, tenant, Some(run_id), None,
            AgentAuditEvent::MicrovmProvisioned,
            "system:controller", Some("system"),
            serde_json::json!({"vm_id": slot.handle.vm_id}),
        ).await?;
        tx.commit().await?;
    }

    // 5. Apply the per-run NetworkPolicy.
    let net_spec = NetworkPolicySpec::strict(tenant.clone(), run_id);
    apply_network_policy(k8s, &net_spec).await?;

    // 6. Transition PROVISIONING → RUNNING + audit RunStarted.
    {
        let mut tx = pg.begin().await?;
        run::update_status(&mut tx, run_id, AgentRunStatus::Running).await?;
        audit::append_agent_audit(
            &mut tx, tenant, Some(run_id), None,
            AgentAuditEvent::RunStarted,
            "system:controller", Some("system"),
            serde_json::json!({"vm_id": slot.handle.vm_id}),
        ).await?;
        tx.commit().await?;
    }

    tracing::info!(%run_id, vm_id = %slot.handle.vm_id, "agent run started");
    Ok(())
}
