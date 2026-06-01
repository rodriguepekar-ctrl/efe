//! **Sprint 8 — N11** : background reaper.
//!
//! One tokio task per pool.  Every `reaper_interval`, the reaper :
//! 1. Health-pings every warm VM ; reaps any that fail.
//! 2. LRU-evicts warm VMs idle past `idle_eviction_after`.
//! 3. Provisions new VMs to satisfy `min_warm` per sub-pool.
//! 4. Updates Prometheus gauges.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::config::FirecrackerPoolConfig;
use super::kind_pool::KindPool;
use super::lifecycle::Lifecycle;
use super::metrics::FirecrackerMetrics;
use super::types::SubPoolKey;

// ─────────────────────────────────────────────────────────────────────
//  Reaper
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : background sweeper.
///
/// Holds a `Weak` to the pool to avoid keeping it alive past
/// `pool.shutdown()`.  Exits cleanly when the parent pool drops.
pub struct Reaper {
    inner: Arc<ReaperInner>,
    task: JoinHandle<()>,
}

pub struct ReaperInner {
    sub_pools: Arc<DashMap<SubPoolKey, Arc<KindPool>>>,
    lifecycle: Arc<dyn Lifecycle>,
    config: Arc<FirecrackerPoolConfig>,
    metrics: Arc<FirecrackerMetrics>,
    /// Signalled by `Reaper::trigger_sweep` to wake the loop early.
    sweep_now: Notify,
    /// Signalled by `Reaper::shutdown` to exit the loop.
    shutdown_signal: Notify,
}

impl Reaper {
    pub fn spawn(
        sub_pools: Arc<DashMap<SubPoolKey, Arc<KindPool>>>,
        lifecycle: Arc<dyn Lifecycle>,
        config: Arc<FirecrackerPoolConfig>,
        metrics: Arc<FirecrackerMetrics>,
    ) -> Self {
        let inner = Arc::new(ReaperInner {
            sub_pools,
            lifecycle,
            config,
            metrics,
            sweep_now: Notify::new(),
            shutdown_signal: Notify::new(),
        });
        let inner_for_task = Arc::clone(&inner);
        let task = tokio::spawn(async move {
            Self::run(inner_for_task).await;
        });
        Self { inner, task }
    }

    /// Wake the reaper loop ahead of its next tick.  Used by tests +
    /// in production by integration points that know a sweep is due
    /// (e.g. after a known spike of crashes).
    pub fn trigger_sweep(&self) {
        self.inner.sweep_now.notify_one();
    }

    /// Gracefully stop the reaper.  Returns after the loop exits.
    pub async fn shutdown(self) -> Result<(), tokio::task::JoinError> {
        self.inner.shutdown_signal.notify_one();
        self.task.await
    }

    /// Stop without waiting (panic-safe).
    pub fn abort(self) {
        self.task.abort();
    }

    /// Main loop.  Holds a `Weak` reference to detect pool drop.
    async fn run(inner: Arc<ReaperInner>) {
        let weak = Arc::downgrade(&inner);
        let mut ticker = tokio::time::interval(inner.config.reaper_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = inner.sweep_now.notified() => {}
                _ = inner.shutdown_signal.notified() => {
                    tracing::info!(
                        target: "efe_nano::firecracker::reaper",
                        "shutdown requested ; exiting loop"
                    );
                    break;
                }
            }
            let Some(inner) = weak.upgrade() else { break };
            Self::sweep_once(&inner).await;
        }
    }

    /// One pass over all sub-pools.  Public for unit tests that drive
    /// the sweep deterministically (no real timer).
    pub async fn sweep_once(inner: &Arc<ReaperInner>) {
        let snapshot: Vec<(SubPoolKey, Arc<KindPool>)> = inner
            .sub_pools
            .iter()
            .map(|kv| (kv.key().clone(), Arc::clone(kv.value())))
            .collect();

        for (key, pool) in snapshot {
            // 1. Health-check warm VMs ; reap the dead.
            let warm_snapshot = pool.snapshot_warm().await;
            for vm in warm_snapshot {
                let alive = inner.lifecycle.is_alive(&vm).await;
                if !alive {
                    tracing::warn!(
                        target: "efe_nano::firecracker::reaper",
                        vm_id = %vm.id,
                        kind = ?key,
                        "warm VM failed health-check ; reaping"
                    );
                    let metrics = pool.metrics();
                    metrics.health_check_failure_total.inc();
                    if let Some(vm) = pool.remove_from_warm(vm.id).await {
                        if let Err(e) = inner.lifecycle.reap(&vm).await {
                            tracing::warn!(
                                target: "efe_nano::firecracker::reaper",
                                vm_id = %vm.id,
                                error = ?e,
                                "reap failed ; will retry next sweep"
                            );
                            metrics.crash_total.inc();
                        } else {
                            metrics.reap_total.inc();
                            inner.metrics.reap_total_global.inc();
                        }
                    }
                }
            }

            // 2. LRU-evict idle warm VMs (down to min_warm).
            let evicted = pool
                .drain_idle_warm(inner.config.idle_eviction_after, pool.config.min_warm)
                .await;
            for vm in evicted {
                if let Err(e) = inner.lifecycle.reap(&vm).await {
                    tracing::warn!(
                        target: "efe_nano::firecracker::reaper",
                        vm_id = %vm.id,
                        error = ?e,
                        "idle-eviction reap failed"
                    );
                }
            }

            // 3. Provision to satisfy min_warm.
            let current_warm = pool.warm_count().await;
            if current_warm < pool.config.min_warm {
                let deficit = pool.config.min_warm - current_warm;
                for _ in 0..deficit {
                    if let Err(e) = pool.provision_one(&inner.lifecycle).await {
                        tracing::warn!(
                            target: "efe_nano::firecracker::reaper",
                            kind = ?key,
                            error = ?e,
                            "background provision failed"
                        );
                        // Don't keep retrying in this sweep — wait for next tick.
                        break;
                    }
                    inner.metrics.spawn_total.inc();
                }
            }
        }
    }
}

#[cfg(test)]
impl ReaperInner {
    /// Hook for tests to manually drive sweeps without spawning a loop.
    pub async fn sweep_for_test(self: &Arc<Self>) {
        Reaper::sweep_once(self).await;
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::KindConfig;
    use super::super::lifecycle::MockLifecycle;
    use super::super::metrics::SubPoolMetrics;
    use super::super::types::TenantClass;
    use std::time::{Duration, Instant};

    fn make_setup(min_warm: u32, max_total: u32) -> (Arc<ReaperInner>, Arc<KindPool>, Arc<MockLifecycle>) {
        let key = SubPoolKey::mode_b("test", "1.0.0", TenantClass::Tier1);
        let mut config = KindConfig::testing_mode_b("test", "1.0.0");
        config.min_warm = min_warm;
        config.max_total = max_total;
        let pool_metrics = Arc::new(SubPoolMetrics::default());
        let pool = Arc::new(KindPool::new(key.clone(), config, pool_metrics));
        let sub_pools = Arc::new(DashMap::new());
        sub_pools.insert(key, Arc::clone(&pool));

        let mock_lc = Arc::new(MockLifecycle::new());
        let lifecycle: Arc<dyn Lifecycle> = Arc::clone(&mock_lc) as Arc<dyn Lifecycle>;
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.reaper_interval = Duration::from_millis(100);
        cfg.idle_eviction_after = Duration::from_secs(300);
        let metrics = Arc::new(FirecrackerMetrics::new());
        let inner = Arc::new(ReaperInner {
            sub_pools,
            lifecycle,
            config: Arc::new(cfg),
            metrics,
            sweep_now: Notify::new(),
            shutdown_signal: Notify::new(),
        });
        (inner, pool, mock_lc)
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sweep : min_warm enforcement
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn sweep_provisions_to_meet_min_warm() {
        let (inner, pool, _mock) = make_setup(3, 10);
        assert_eq!(pool.warm_count().await, 0);
        inner.sweep_for_test().await;
        assert_eq!(pool.warm_count().await, 3, "reaper must provision min_warm");
    }

    #[tokio::test]
    async fn sweep_does_not_exceed_min_warm() {
        let (inner, pool, _) = make_setup(2, 10);
        for _ in 0..3 {
            inner.sweep_for_test().await;
        }
        // 3 sweeps — still only 2 warm.
        assert_eq!(pool.warm_count().await, 2);
    }

    #[tokio::test]
    async fn sweep_handles_spawn_failures_gracefully() {
        let (inner, pool, mock) = make_setup(2, 10);
        mock.fail_next_spawns(1);
        inner.sweep_for_test().await;
        // First spawn fails, reaper breaks the inner loop, so 0 added.
        assert_eq!(pool.warm_count().await, 0);
        // Next sweep should recover.
        inner.sweep_for_test().await;
        assert_eq!(pool.warm_count().await, 2);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sweep : health-check + reap
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn sweep_reaps_dead_warm_vm() {
        let (inner, pool, mock) = make_setup(2, 10);
        inner.sweep_for_test().await;
        assert_eq!(pool.warm_count().await, 2);

        // Kill one of them.
        let snap = pool.snapshot_warm().await;
        let victim_id = snap[0].id;
        mock.kill_vm(victim_id);

        inner.sweep_for_test().await;
        // The dead VM is reaped + replaced.
        let after = pool.snapshot_warm().await;
        assert_eq!(after.len(), 2);
        assert!(
            after.iter().all(|v| v.id != victim_id),
            "dead VM must not be in warm-set after sweep"
        );
    }

    #[tokio::test]
    async fn sweep_metrics_counters_increment() {
        let (inner, pool, mock) = make_setup(2, 10);
        inner.sweep_for_test().await;
        let snap = pool.snapshot_warm().await;
        mock.kill_vm(snap[0].id);
        inner.sweep_for_test().await;

        let m = pool.metrics();
        assert!(m.health_check_failure_total.get() >= 1);
        assert!(m.reap_total.get() >= 1);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sweep : idle eviction
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn sweep_evicts_idle_warm_when_above_min() {
        // Cold init : 4 warm but min_warm = 2 + idle_eviction_after = 0.
        let (inner, pool, _) = make_setup(2, 10);
        // Bypass reaper to seed 4 warm directly.
        let lc: Arc<dyn Lifecycle> = Arc::clone(&inner.lifecycle);
        for _ in 0..4 {
            pool.provision_one(&lc).await.unwrap();
        }
        assert_eq!(pool.warm_count().await, 4);

        // Set eviction to immediate so the test doesn't wait.
        let inner_for_evict = Arc::new(ReaperInner {
            sub_pools: Arc::clone(&inner.sub_pools),
            lifecycle: Arc::clone(&inner.lifecycle),
            config: Arc::new({
                let mut cfg = (*inner.config).clone();
                cfg.idle_eviction_after = Duration::ZERO;
                cfg
            }),
            metrics: Arc::clone(&inner.metrics),
            sweep_now: Notify::new(),
            shutdown_signal: Notify::new(),
        });
        inner_for_evict.sweep_for_test().await;
        // Down to min_warm = 2.
        assert_eq!(pool.warm_count().await, 2);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Reaper spawn / shutdown lifecycle
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reaper_spawns_and_provisions_in_background() {
        let (inner, pool, _) = make_setup(2, 10);
        let reaper = Reaper::spawn(
            Arc::clone(&inner.sub_pools),
            Arc::clone(&inner.lifecycle),
            Arc::clone(&inner.config),
            Arc::clone(&inner.metrics),
        );
        // Trigger an immediate sweep instead of waiting for the tick.
        reaper.trigger_sweep();
        // Wait a few ms for the sweep to land.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(pool.warm_count().await >= 1);
        reaper.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reaper_shutdown_terminates_loop() {
        let (inner, _pool, _) = make_setup(0, 10);
        let reaper = Reaper::spawn(
            Arc::clone(&inner.sub_pools),
            Arc::clone(&inner.lifecycle),
            Arc::clone(&inner.config),
            Arc::clone(&inner.metrics),
        );
        let start = Instant::now();
        reaper.shutdown().await.unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "shutdown must be near-immediate"
        );
    }

    #[tokio::test]
    async fn reaper_abort_does_not_hang() {
        let (inner, _pool, _) = make_setup(0, 10);
        let reaper = Reaper::spawn(
            Arc::clone(&inner.sub_pools),
            Arc::clone(&inner.lifecycle),
            Arc::clone(&inner.config),
            Arc::clone(&inner.metrics),
        );
        reaper.abort();
    }
}
