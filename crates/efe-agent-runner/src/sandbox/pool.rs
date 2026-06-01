//! Per-tenant warm pool of microVMs.
//!
//! Cold-start of Firecracker is ~125 ms ; we maintain a small pre-warmed
//! pool per tenant so AgentRun creation is sub-second wall-clock. Pool
//! size is configured per tenant in `tenant_feature.m25_microvm_pool_size`
//! (default 2).
//!
//! Eviction: LRU. Drain on tenant disable.

use crate::sandbox::firecracker::{
    allocate_vsock_cid, new_vm_id, FirecrackerDriver, FirecrackerHandle,
    MicrovmConfig, SeccompProfile,
};
use crate::sandbox::SandboxResources;
use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Idle,      // ready to accept a run
    Running,   // currently bound to a run
    Draining,  // marked for shutdown after current run
}

#[derive(Debug, Clone)]
pub struct MicrovmSlot {
    pub handle: FirecrackerHandle,
    pub state: SlotState,
    pub last_used: Instant,
    pub run_count: u32,
}

/// Configuration knobs for the pool ; loaded from tenant flags.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub target_size: u32,
    pub max_age: Duration,           // recycle after N hours even if idle
    pub max_runs_per_vm: u32,        // recycle after N runs (defense-in-depth)
    pub spawn_timeout: Duration,
    pub resources: SandboxResources,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            target_size: 2,
            max_age: Duration::from_secs(3600),  // 1h
            max_runs_per_vm: 100,
            spawn_timeout: Duration::from_secs(30),
            resources: SandboxResources::default(),
        }
    }
}

/// Per-tenant pool. Internally guarded by a single mutex so we can
/// keep the impl simple — at design-partner pilot scale (single tenant,
/// max 5 concurrent runs) contention is irrelevant.
pub struct MicrovmPool {
    tenant: TenantId,
    cfg: PoolConfig,
    driver: Arc<dyn FirecrackerDriver>,
    slots: Arc<Mutex<VecDeque<MicrovmSlot>>>,
}

impl MicrovmPool {
    pub fn new(
        tenant: TenantId,
        cfg: PoolConfig,
        driver: Arc<dyn FirecrackerDriver>,
    ) -> Self {
        Self {
            tenant,
            cfg,
            driver,
            slots: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Lease a slot. Returns the handle and a `SlotGuard` that releases
    /// (back to Idle) on drop, or marks for drain if `draining=true`.
    pub async fn lease(&self) -> AgentRunnerResult<MicrovmSlot> {
        let mut slots = self.slots.lock().await;

        // Find an idle slot first.
        if let Some(idx) = slots.iter().position(|s| s.state == SlotState::Idle) {
            let mut slot = slots.remove(idx).unwrap();
            slot.state = SlotState::Running;
            slot.last_used = Instant::now();
            slot.run_count += 1;
            slots.push_back(slot.clone());

            // Update the metrics gauge.
            crate::metrics::AGENT_MICROVM_POOL_SIZE
                .with_label_values(&[&self.tenant.0, "running"])
                .inc();
            crate::metrics::AGENT_MICROVM_POOL_SIZE
                .with_label_values(&[&self.tenant.0, "idle"])
                .dec();

            return Ok(slot);
        }

        // No idle slot — spawn one on demand if below target_size + 1.
        let total = slots.len() as u32;
        if total >= self.cfg.target_size + 1 {
            return Err(AgentRunnerError::MaxConcurrentRunsReached {
                tenant: self.tenant.0.clone(),
                current: total,
                max: self.cfg.target_size + 1,
            });
        }

        let cfg = MicrovmConfig {
            vm_id: new_vm_id(),
            resources: self.cfg.resources.clone(),
            vsock_cid: allocate_vsock_cid(rand::random::<u32>()),
            network_iface: "tap-efe0".into(),
            seccomp_profile: SeccompProfile::AgentCore,
        };
        let started = Instant::now();
        let handle = self.driver.spawn(&cfg).await?;
        let took_ms = started.elapsed().as_millis() as u64;
        crate::metrics::AGENT_MICROVM_PROVISION_DURATION
            .with_label_values(&[&self.tenant.0, "ok"])
            .observe(started.elapsed().as_secs_f64());

        let slot = MicrovmSlot {
            handle,
            state: SlotState::Running,
            last_used: Instant::now(),
            run_count: 1,
        };
        slots.push_back(slot.clone());

        crate::metrics::AGENT_MICROVM_POOL_SIZE
            .with_label_values(&[&self.tenant.0, "running"])
            .inc();

        tracing::info!(
            tenant = %self.tenant.0,
            vm_id = %cfg.vm_id,
            took_ms,
            "microvm spawned (cold)"
        );

        Ok(slot)
    }

    /// Release a slot back to Idle (or drain if recycling needed).
    pub async fn release(&self, vm_id: &str) -> AgentRunnerResult<()> {
        let mut slots = self.slots.lock().await;
        let pos = slots.iter().position(|s| s.handle.vm_id == vm_id);
        let idx = match pos {
            Some(i) => i,
            None => return Ok(()), // already drained
        };
        let mut slot = slots.remove(idx).unwrap();

        let too_old = slot.last_used.elapsed() >= self.cfg.max_age;
        let too_many_runs = slot.run_count >= self.cfg.max_runs_per_vm;

        if too_old || too_many_runs {
            slot.state = SlotState::Draining;
            tracing::info!(
                vm_id = %slot.handle.vm_id,
                age_s = slot.last_used.elapsed().as_secs(),
                run_count = slot.run_count,
                "microvm draining (recycle)"
            );
            self.driver.kill(&slot.handle.vm_id).await?;
            crate::metrics::AGENT_MICROVM_POOL_SIZE
                .with_label_values(&[&self.tenant.0, "running"])
                .dec();
            // Don't push back — slot is gone.
            return Ok(());
        }

        slot.state = SlotState::Idle;
        slot.last_used = Instant::now();
        slots.push_back(slot);

        crate::metrics::AGENT_MICROVM_POOL_SIZE
            .with_label_values(&[&self.tenant.0, "running"])
            .dec();
        crate::metrics::AGENT_MICROVM_POOL_SIZE
            .with_label_values(&[&self.tenant.0, "idle"])
            .inc();
        Ok(())
    }

    /// Background task : keep `target_size` warm slots ready. Spawned
    /// once per tenant.
    pub async fn warm_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let slots = self.slots.lock().await;
            let idle_count = slots.iter().filter(|s| s.state == SlotState::Idle).count() as u32;
            let total = slots.len() as u32;
            if idle_count < self.cfg.target_size && total < self.cfg.target_size {
                let need = self.cfg.target_size - total;
                drop(slots);
                for _ in 0..need {
                    let cfg = MicrovmConfig {
                        vm_id: new_vm_id(),
                        resources: self.cfg.resources.clone(),
                        vsock_cid: allocate_vsock_cid(rand::random::<u32>()),
                        network_iface: "tap-efe0".into(),
                        seccomp_profile: SeccompProfile::AgentCore,
                    };
                    match self.driver.spawn(&cfg).await {
                        Ok(handle) => {
                            let slot = MicrovmSlot {
                                handle,
                                state: SlotState::Idle,
                                last_used: Instant::now(),
                                run_count: 0,
                            };
                            self.slots.lock().await.push_back(slot);
                            crate::metrics::AGENT_MICROVM_POOL_SIZE
                                .with_label_values(&[&self.tenant.0, "idle"])
                                .inc();
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to warm microvm");
                        }
                    }
                }
            }
        }
    }

    /// Drain everything (tenant disabled).
    pub async fn drain_all(&self) -> AgentRunnerResult<()> {
        let mut slots = self.slots.lock().await;
        for slot in slots.iter() {
            let _ = self.driver.kill(&slot.handle.vm_id).await;
        }
        slots.clear();
        Ok(())
    }

    /// M26 — return vm_ids of `Running` slots that are NOT in the
    /// `in_use` set AND whose `last_used` is older than `max_age`.
    /// Used by the orphan slot reaper.
    pub async fn list_stale(
        &self,
        in_use: &std::collections::HashSet<String>,
        max_age: std::time::Duration,
    ) -> Vec<String> {
        let slots = self.slots.lock().await;
        let now = Instant::now();
        slots
            .iter()
            .filter(|s| s.state == SlotState::Running)
            .filter(|s| !in_use.contains(&s.handle.vm_id))
            .filter(|s| now.duration_since(s.last_used) >= max_age)
            .map(|s| s.handle.vm_id.clone())
            .collect()
    }

    /// M26 — test-only snapshot of slots (used by lease_guard tests).
    #[cfg(any(test, feature = "test-utils"))]
    pub async fn slots_for_test(&self) -> Vec<MicrovmSlot> {
        self.slots.lock().await.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::firecracker::MockFirecrackerDriver;

    fn mk_pool(target: u32) -> MicrovmPool {
        MicrovmPool::new(
            TenantId("t1".into()),
            PoolConfig { target_size: target, ..Default::default() },
            Arc::new(MockFirecrackerDriver::new()),
        )
    }

    #[tokio::test]
    async fn lease_spawns_when_pool_empty() {
        let pool = mk_pool(1);
        let slot = pool.lease().await.unwrap();
        assert_eq!(slot.state, SlotState::Running);
        assert_eq!(slot.run_count, 1);
    }

    #[tokio::test]
    async fn release_returns_to_idle() {
        let pool = mk_pool(1);
        let slot = pool.lease().await.unwrap();
        pool.release(&slot.handle.vm_id).await.unwrap();
        let s2 = pool.lease().await.unwrap();
        assert_eq!(s2.handle.vm_id, slot.handle.vm_id, "should reuse the same VM");
        assert_eq!(s2.run_count, 2);
    }

    #[tokio::test]
    async fn lease_caps_at_target_plus_one() {
        let pool = mk_pool(1);
        let _s1 = pool.lease().await.unwrap();
        let _s2 = pool.lease().await.unwrap();
        let err = pool.lease().await.unwrap_err();
        assert!(matches!(err, AgentRunnerError::MaxConcurrentRunsReached { .. }));
    }

    #[tokio::test]
    async fn drain_all_kills_everything() {
        let pool = mk_pool(2);
        let _s = pool.lease().await.unwrap();
        pool.drain_all().await.unwrap();
        let slots = pool.slots.lock().await;
        assert!(slots.is_empty());
    }
}
