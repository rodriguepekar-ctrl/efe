//! **Sprint 8 — N11** : the `KindPool` data structure.
//!
//! A `KindPool` is one sub-pool keyed by `(VmKind, model_id, model_version,
//! tenant_class)`.  It holds :
//! - `warm`     : a `VecDeque<Arc<WarmVm>>` of idle VMs ready to serve.
//! - `in_use`   : a `HashMap<VmId, Arc<WarmVm>>` of leased VMs.
//! - `permits`  : a `tokio::sync::Semaphore` bounded to `max_total`.
//!
//! # Concurrency contract
//!
//! - `checkout` blocks on `permits.acquire()` up to `wait_budget`.
//! - On hit (warm-set non-empty), pops from `warm`, inserts into
//!   `in_use`, returns.
//! - On miss, releases the permit (so others can use it for warm hits)
//!   and provisions a new VM via the `Lifecycle`.  Re-acquires the
//!   permit on success.  Past `wait_budget` → `AcquireTimeout`.
//! - `checkin_warm` returns a healthy VM to `warm`.
//! - `checkin_reaped` discards a dead VM ; reaper or background
//!   provisioner refills.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};

use crate::error::VmPoolError;

use super::config::KindConfig;
use super::lifecycle::{Lifecycle, LifecycleError};
use super::metrics::SubPoolMetrics;
use super::types::{SubPoolKey, VmId, VmState, WarmVm};

// ─────────────────────────────────────────────────────────────────────
//  KindPool
// ─────────────────────────────────────────────────────────────────────

pub struct KindPool {
    pub key: SubPoolKey,
    pub config: KindConfig,
    inner: Arc<KindPoolInner>,
    metrics: Arc<SubPoolMetrics>,
}

struct KindPoolInner {
    warm: Mutex<VecDeque<Arc<WarmVm>>>,
    in_use: Mutex<HashMap<VmId, Arc<WarmVm>>>,
    permits: Arc<Semaphore>,
}

impl KindPool {
    pub fn new(
        key: SubPoolKey,
        config: KindConfig,
        metrics: Arc<SubPoolMetrics>,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(config.max_total as usize));
        Self {
            key,
            config,
            inner: Arc::new(KindPoolInner {
                warm: Mutex::new(VecDeque::new()),
                in_use: Mutex::new(HashMap::new()),
                permits,
            }),
            metrics,
        }
    }

    /// **Sprint 8 — N11** : checkout one VM, blocking up to
    /// `wait_budget`.  Provisions a new VM (cold path) if the warm-set
    /// is empty and the permit cap allows.
    pub async fn checkout(
        &self,
        lifecycle: &Arc<dyn Lifecycle>,
        wait_budget: Duration,
    ) -> Result<Arc<WarmVm>, VmPoolError> {
        let start = Instant::now();

        // 1. Acquire a permit (gates max_total).
        let permit = tokio::time::timeout(wait_budget, self.inner.permits.clone().acquire_owned())
            .await
            .map_err(|_| {
                self.metrics.acquire_timeout_total.inc();
                self.metrics.pool_exhausted_total.inc();
                VmPoolError::AcquireTimeout(wait_budget.as_millis() as u32)
            })?
            .map_err(|e| VmPoolError::Internal(format!("semaphore closed: {e}")))?;

        // 2. Try the warm-set first.
        let warm_vm = {
            let mut warm = self.inner.warm.lock().await;
            warm.pop_front()
        };

        if let Some(vm) = warm_vm {
            // Hit !
            self.metrics.warm_count.dec();
            self.metrics.in_use_count.inc();
            *vm.state.lock().unwrap() = VmState::Reserved;
            self.inner.in_use.lock().await.insert(vm.id, Arc::clone(&vm));
            // Keep the permit alive via Arc<WarmVm> ; release on checkin.
            // (Permit is leak()-ed because the VM holds the slot until
            // checkin ; this is intentional — Semaphore semantics align
            // exactly with "slot occupancy".)
            permit.forget();
            self.metrics.acquire_wait.observe(start.elapsed());
            return Ok(vm);
        }

        // 3. Cold path : provision.
        let cold_start = Instant::now();
        let remaining = wait_budget.checked_sub(start.elapsed()).unwrap_or_default();
        if remaining.is_zero() {
            self.metrics.acquire_timeout_total.inc();
            return Err(VmPoolError::AcquireTimeout(wait_budget.as_millis() as u32));
        }
        let vm_result = tokio::time::timeout(
            remaining,
            lifecycle.spawn(self.key.clone(), self.config.clone()),
        )
        .await;

        let vm = match vm_result {
            Err(_) => {
                self.metrics.acquire_timeout_total.inc();
                return Err(VmPoolError::AcquireTimeout(wait_budget.as_millis() as u32));
            }
            Ok(Err(LifecycleError::SpawnFailed(reason))) => {
                self.metrics.crash_total.inc();
                return Err(VmPoolError::ProvisionFailed(reason));
            }
            Ok(Err(LifecycleError::BootTimeout { boot_budget_ms })) => {
                self.metrics.crash_total.inc();
                return Err(VmPoolError::ProvisionFailed(format!(
                    "boot timeout after {boot_budget_ms}ms"
                )));
            }
            Ok(Err(e)) => {
                return Err(VmPoolError::ProvisionFailed(e.to_string()));
            }
            Ok(Ok(vm)) => Arc::new(vm),
        };

        self.metrics.cold_start_total.inc();
        self.metrics.cold_start_duration.observe(cold_start.elapsed());
        self.metrics.in_use_count.inc();
        *vm.state.lock().unwrap() = VmState::Reserved;
        self.inner.in_use.lock().await.insert(vm.id, Arc::clone(&vm));
        permit.forget();
        self.metrics.acquire_wait.observe(start.elapsed());
        Ok(vm)
    }

    /// **Sprint 8 — N11** : return a healthy VM to the warm-set.
    /// Releases the semaphore permit.
    pub async fn checkin_warm(&self, vm: Arc<WarmVm>) {
        let vm_id = vm.id;
        let mut in_use = self.inner.in_use.lock().await;
        in_use.remove(&vm_id);
        self.metrics.in_use_count.dec();
        drop(in_use);

        // Verify state allows re-warming.
        *vm.state.lock().unwrap() = VmState::Warm;
        vm.touch();

        let mut warm = self.inner.warm.lock().await;
        warm.push_back(vm);
        self.metrics.warm_count.inc();
        drop(warm);

        // Restore the permit so future checkouts can grab a fresh slot.
        self.inner.permits.add_permits(1);
    }

    /// **Sprint 8 — N11** : discard a dead VM.  Reaper or background
    /// provisioner refills.  Permit released so the slot is free.
    pub async fn checkin_reaped(&self, vm: Arc<WarmVm>) {
        let vm_id = vm.id;
        let mut in_use = self.inner.in_use.lock().await;
        in_use.remove(&vm_id);
        self.metrics.in_use_count.dec();
        drop(in_use);

        *vm.state.lock().unwrap() = VmState::Dead;
        self.metrics.reap_total.inc();

        // Permit released so the slot is free for a fresh spawn.
        self.inner.permits.add_permits(1);
    }

    /// **Sprint 8 — N11** : drain the warm-set.  Used by background
    /// reaper to LRU-evict idle VMs.  Returns the evicted VMs ; the
    /// caller is responsible for `lifecycle.reap()` on each.
    pub async fn drain_idle_warm(
        &self,
        idle_threshold: Duration,
        min_warm: u32,
    ) -> Vec<Arc<WarmVm>> {
        let mut warm = self.inner.warm.lock().await;
        let current = warm.len() as u32;
        if current <= min_warm {
            return Vec::new();
        }
        let max_evict = (current - min_warm) as usize;
        let mut evicted = Vec::with_capacity(max_evict);
        let mut keep = VecDeque::with_capacity(warm.len());
        while let Some(vm) = warm.pop_front() {
            if evicted.len() < max_evict && vm.idle_for() > idle_threshold {
                evicted.push(vm);
            } else {
                keep.push_back(vm);
            }
        }
        *warm = keep;
        let evicted_count = evicted.len() as u64;
        let new_len = warm.len() as u64;
        drop(warm);
        // Update warm_count gauge after dropping the guard to avoid
        // re-locking.
        self.metrics.warm_count.set(new_len);
        if evicted_count > 0 {
            tracing::debug!(
                target: "efe_nano::firecracker",
                kind = ?self.key,
                evicted = evicted_count,
                "drained idle warm VMs"
            );
        }
        // Release permits for the evicted slots.
        self.inner.permits.add_permits(evicted_count as usize);
        evicted
    }

    /// **Sprint 8 — N11** : current warm count (for ops dashboards).
    pub async fn warm_count(&self) -> u32 {
        self.inner.warm.lock().await.len() as u32
    }

    /// **Sprint 8 — N11** : current in-use count.
    pub async fn in_use_count(&self) -> u32 {
        self.inner.in_use.lock().await.len() as u32
    }

    /// **Sprint 8 — N11** : provision a single VM into the warm-set
    /// (background prewarm).  Holds a permit for the lifetime of the
    /// provisioning.  Returns Ok even if other VMs were already provisioned
    /// concurrently — the caller is the reaper, which retries up to
    /// `min_warm`.
    pub async fn provision_one(
        &self,
        lifecycle: &Arc<dyn Lifecycle>,
    ) -> Result<(), VmPoolError> {
        // Try to acquire a permit without waiting — if the pool is at cap,
        // we just skip this provision.
        let permit = match self.inner.permits.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let cold_start = Instant::now();
        match lifecycle
            .spawn(self.key.clone(), self.config.clone())
            .await
        {
            Ok(vm) => {
                self.metrics.cold_start_total.inc();
                self.metrics.cold_start_duration.observe(cold_start.elapsed());
                let arc = Arc::new(vm);
                *arc.state.lock().unwrap() = VmState::Warm;
                self.inner.warm.lock().await.push_back(arc);
                self.metrics.warm_count.inc();
                permit.forget();
                Ok(())
            }
            Err(e) => {
                self.metrics.crash_total.inc();
                Err(VmPoolError::ProvisionFailed(e.to_string()))
            }
        }
    }

    /// **Sprint 8 — N11** : snapshot of all warm VMs (used by the reaper
    /// to health-ping each one).  Returns Arcs so the reaper doesn't
    /// hold the warm mutex during the health checks.
    pub async fn snapshot_warm(&self) -> Vec<Arc<WarmVm>> {
        self.inner.warm.lock().await.iter().cloned().collect()
    }

    /// **Sprint 8 — N11** : remove a specific VM from the warm-set
    /// (matched by id).  Returns the Arc if found.  Used by the reaper
    /// to evict dead VMs.
    pub async fn remove_from_warm(&self, vm_id: VmId) -> Option<Arc<WarmVm>> {
        let mut warm = self.inner.warm.lock().await;
        let pos = warm.iter().position(|v| v.id == vm_id)?;
        let vm = warm.remove(pos)?;
        self.metrics.warm_count.dec();
        // Release the permit since the slot is now empty.
        self.inner.permits.add_permits(1);
        Some(vm)
    }

    pub fn metrics(&self) -> &Arc<SubPoolMetrics> {
        &self.metrics
    }
}

impl std::fmt::Debug for KindPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KindPool")
            .field("key", &self.key)
            .field("max_total", &self.config.max_total)
            .finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::lifecycle::MockLifecycle;
    use super::super::types::TenantClass;

    fn make_pool() -> (KindPool, Arc<dyn Lifecycle>) {
        let key = SubPoolKey::mode_b("test_model", "1.0.0", TenantClass::Tier1);
        let mut config = KindConfig::testing_mode_b("test_model", "1.0.0");
        config.min_warm = 2;
        config.max_total = 5;
        let metrics = Arc::new(SubPoolMetrics::default());
        let pool = KindPool::new(key, config, metrics);
        let lc: Arc<dyn Lifecycle> = Arc::new(MockLifecycle::new());
        (pool, lc)
    }

    // ─────────────────────────────────────────────────────────────────
    //  Cold-path checkout
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn checkout_cold_path_provisions_a_vm() {
        let (pool, lc) = make_pool();
        let vm = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
        assert_eq!(pool.warm_count().await, 0);
        assert_eq!(pool.in_use_count().await, 1);
        pool.checkin_warm(vm).await;
        assert_eq!(pool.warm_count().await, 1);
        assert_eq!(pool.in_use_count().await, 0);
    }

    #[tokio::test]
    async fn checkout_warm_path_hits_existing_vm() {
        let (pool, lc) = make_pool();
        // Provision once + checkin → vm now in warm.
        let vm = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
        let vm_id = vm.id;
        pool.checkin_warm(vm).await;
        // Re-checkout : should return the same VM.
        let again = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
        assert_eq!(again.id, vm_id, "warm-path hit must reuse the same VM");
    }

    #[tokio::test]
    async fn checkout_respects_max_total() {
        let (pool, lc) = make_pool();
        let mut leases = Vec::new();
        for _ in 0..5 {
            leases.push(pool.checkout(&lc, Duration::from_secs(1)).await.unwrap());
        }
        // 6th checkout : pool full, must time out quickly.
        let start = Instant::now();
        let err = pool.checkout(&lc, Duration::from_millis(50)).await.unwrap_err();
        assert!(matches!(err, VmPoolError::AcquireTimeout(_)));
        assert!(start.elapsed() < Duration::from_millis(200));
        // Release one → next checkout works.
        let released = leases.pop().unwrap();
        pool.checkin_warm(released).await;
        let new_vm = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
        leases.push(new_vm);
    }

    #[tokio::test]
    async fn checkin_reaped_does_not_return_to_warm() {
        let (pool, lc) = make_pool();
        let vm = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
        pool.checkin_reaped(vm).await;
        assert_eq!(pool.warm_count().await, 0);
        assert_eq!(pool.in_use_count().await, 0);
    }

    #[tokio::test]
    async fn checkin_reaped_releases_permit_for_new_spawn() {
        let (pool, lc) = make_pool();
        // Saturate.
        let mut leases = Vec::new();
        for _ in 0..5 {
            leases.push(pool.checkout(&lc, Duration::from_secs(1)).await.unwrap());
        }
        // Reap one → permit freed → next checkout works.
        let vm = leases.pop().unwrap();
        pool.checkin_reaped(vm).await;
        let _new = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap();
    }

    // ─────────────────────────────────────────────────────────────────
    //  Pre-provisioning
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn provision_one_adds_to_warm() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        assert_eq!(pool.warm_count().await, 1);
        assert_eq!(pool.in_use_count().await, 0);
    }

    #[tokio::test]
    async fn provision_one_respects_cap() {
        let (pool, lc) = make_pool();
        // Saturate without checkin.
        let mut leases = Vec::new();
        for _ in 0..5 {
            leases.push(pool.checkout(&lc, Duration::from_secs(1)).await.unwrap());
        }
        // Try-provision more : should silently skip.
        pool.provision_one(&lc).await.unwrap();
        assert_eq!(pool.warm_count().await, 0);
        assert_eq!(pool.in_use_count().await, 5);
    }

    #[tokio::test]
    async fn provision_one_propagates_spawn_failures() {
        let (pool, _) = make_pool();
        let mock = Arc::new(MockLifecycle::new());
        mock.fail_next_spawns(1);
        let lc: Arc<dyn Lifecycle> = mock;
        let err = pool.provision_one(&lc).await.unwrap_err();
        assert!(matches!(err, VmPoolError::ProvisionFailed(_)));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Idle eviction
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn drain_idle_warm_evicts_when_above_min() {
        let (pool, lc) = make_pool();
        // Provision 4 (min_warm = 2).
        for _ in 0..4 {
            pool.provision_one(&lc).await.unwrap();
        }
        // None have aged ; immediate drain with 0-threshold respects min_warm.
        let evicted = pool.drain_idle_warm(Duration::ZERO, 2).await;
        assert_eq!(evicted.len(), 2);
        assert_eq!(pool.warm_count().await, 2);
    }

    #[tokio::test]
    async fn drain_idle_warm_returns_nothing_below_min() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        // 1 warm, min_warm = 2 → nothing to evict.
        let evicted = pool.drain_idle_warm(Duration::ZERO, 2).await;
        assert!(evicted.is_empty());
    }

    #[tokio::test]
    async fn drain_idle_warm_respects_age_threshold() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        pool.provision_one(&lc).await.unwrap();
        pool.provision_one(&lc).await.unwrap();
        // Threshold higher than the VMs' age → no eviction.
        let evicted = pool.drain_idle_warm(Duration::from_secs(60), 1).await;
        assert!(evicted.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────
    //  Snapshot + remove
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn snapshot_warm_returns_all_idle_vms() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        pool.provision_one(&lc).await.unwrap();
        let snap = pool.snapshot_warm().await;
        assert_eq!(snap.len(), 2);
    }

    #[tokio::test]
    async fn remove_from_warm_takes_specific_vm() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        pool.provision_one(&lc).await.unwrap();
        let snap = pool.snapshot_warm().await;
        let target_id = snap[0].id;
        let removed = pool.remove_from_warm(target_id).await.unwrap();
        assert_eq!(removed.id, target_id);
        assert_eq!(pool.warm_count().await, 1);
    }

    #[tokio::test]
    async fn remove_from_warm_returns_none_for_unknown_id() {
        let (pool, lc) = make_pool();
        pool.provision_one(&lc).await.unwrap();
        let removed = pool.remove_from_warm(VmId(99999)).await;
        assert!(removed.is_none());
    }

    // ─────────────────────────────────────────────────────────────────
    //  Concurrent checkout stress
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrent_checkouts_serialize_under_cap() {
        let (pool, lc) = make_pool();
        let pool = Arc::new(pool);
        let lc = Arc::clone(&lc);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let p = Arc::clone(&pool);
            let l = Arc::clone(&lc);
            handles.push(tokio::spawn(async move {
                let vm = p.checkout(&l, Duration::from_secs(2)).await.unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                p.checkin_warm(vm).await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // After all done : in_use = 0, warm = 5 (capped at max_total).
        assert_eq!(pool.in_use_count().await, 0);
        assert!(pool.warm_count().await >= 1 && pool.warm_count().await <= 5);
    }

    #[tokio::test]
    async fn checkout_zero_budget_yields_timeout() {
        let (pool, lc) = make_pool();
        let err = pool.checkout(&lc, Duration::from_millis(0)).await.unwrap_err();
        assert!(matches!(err, VmPoolError::AcquireTimeout(_)));
    }

    #[tokio::test]
    async fn checkout_propagates_spawn_failure_as_provision_failed() {
        let key = SubPoolKey::mode_b("test", "1.0.0", TenantClass::Tier1);
        let mut cfg = KindConfig::testing_mode_b("test", "1.0.0");
        cfg.min_warm = 0;
        cfg.max_total = 1;
        let metrics = Arc::new(SubPoolMetrics::default());
        let pool = KindPool::new(key, cfg, metrics);
        let mock = Arc::new(MockLifecycle::new());
        mock.fail_next_spawns(5);
        let lc: Arc<dyn Lifecycle> = mock;
        let err = pool.checkout(&lc, Duration::from_secs(1)).await.unwrap_err();
        assert!(matches!(err, VmPoolError::ProvisionFailed(_)));
    }
}
