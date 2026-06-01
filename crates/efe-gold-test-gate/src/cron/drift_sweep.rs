// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Drift sweep cron loop — § 7.6 + G8 + G9.
//!
//! Architecture decision : Sprint 12 does NOT pull in
//! `tokio-cron-scheduler` (heavy dep with own runtime conflicts).
//! Instead, a simple `tokio::time::interval` loop runs every
//! `tick_interval` (default 24h).  Each tick :
//!
//! 1. Reads `DriftSweepState` to find agents whose `last_run_at` is
//!    older than the drift threshold (default 23h).
//! 2. For each, calls `GoldGate::run` with the latest approved gold
//!    set.
//! 3. On success → updates `last_run_at` ; on failure → emits a
//!    `GoldDriftDetected` ledger row + marks the agent for demotion.
//!
//! Idempotency (G9) : the drift-state per-agent `last_run_at` is the
//! source of truth ; a process restart re-reads it and skips agents
//! < 23h old.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{GoldRunId, NanoAgentId, NanoAgentVersion};
use tokio::sync::Notify;

/// **N20 + G8** : per-agent drift-state row.
#[derive(Debug, Clone, PartialEq)]
pub struct DriftSweepStateEntry {
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub last_run_at: SystemTime,
    pub last_gold_run_id: GoldRunId,
    pub last_outcome: String,
}

/// **N20** : trait abstraction for the drift-state store.
/// Sprint 12 ships `InMemoryDriftSweepState` ; Sprint 13 wires
/// `PostgresDriftSweepState`.
#[async_trait]
pub trait DriftSweepState: Send + Sync + 'static {
    async fn upsert(
        &self,
        entry: DriftSweepStateEntry,
    ) -> Result<(), DriftSweepStateError>;

    async fn last_run_at(
        &self,
        agent_id: &NanoAgentId,
        agent_version: &NanoAgentVersion,
    ) -> Result<Option<SystemTime>, DriftSweepStateError>;

    async fn list_stale(
        &self,
        older_than: Duration,
        now: SystemTime,
    ) -> Result<Vec<DriftSweepStateEntry>, DriftSweepStateError>;

    async fn lookup(
        &self,
        agent_id: &NanoAgentId,
        agent_version: &NanoAgentVersion,
    ) -> Result<Option<DriftSweepStateEntry>, DriftSweepStateError>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DriftSweepStateError {
    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N20** : in-memory implementation.
pub struct InMemoryDriftSweepState {
    rows: DashMap<(NanoAgentId, NanoAgentVersion), DriftSweepStateEntry>,
}

impl Default for InMemoryDriftSweepState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InMemoryDriftSweepState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryDriftSweepState")
            .field("rows", &self.rows.len())
            .finish()
    }
}

impl InMemoryDriftSweepState {
    pub fn new() -> Self {
        Self {
            rows: DashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[async_trait]
impl DriftSweepState for InMemoryDriftSweepState {
    async fn upsert(
        &self,
        entry: DriftSweepStateEntry,
    ) -> Result<(), DriftSweepStateError> {
        let key = (entry.agent_id.clone(), entry.agent_version.clone());
        self.rows.insert(key, entry);
        Ok(())
    }

    async fn last_run_at(
        &self,
        agent_id: &NanoAgentId,
        agent_version: &NanoAgentVersion,
    ) -> Result<Option<SystemTime>, DriftSweepStateError> {
        Ok(self
            .rows
            .get(&(agent_id.clone(), agent_version.clone()))
            .map(|r| r.value().last_run_at))
    }

    async fn list_stale(
        &self,
        older_than: Duration,
        now: SystemTime,
    ) -> Result<Vec<DriftSweepStateEntry>, DriftSweepStateError> {
        let mut out = Vec::new();
        for entry in self.rows.iter() {
            let elapsed = now
                .duration_since(entry.value().last_run_at)
                .unwrap_or_default();
            if elapsed >= older_than {
                out.push(entry.value().clone());
            }
        }
        // Deterministic order : by agent_id then version.
        out.sort_by(|a, b| {
            a.agent_id
                .as_str()
                .cmp(b.agent_id.as_str())
                .then_with(|| a.agent_version.as_str().cmp(b.agent_version.as_str()))
        });
        Ok(out)
    }

    async fn lookup(
        &self,
        agent_id: &NanoAgentId,
        agent_version: &NanoAgentVersion,
    ) -> Result<Option<DriftSweepStateEntry>, DriftSweepStateError> {
        Ok(self
            .rows
            .get(&(agent_id.clone(), agent_version.clone()))
            .map(|r| r.value().clone()))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DriftSweepCron — runtime task
// ─────────────────────────────────────────────────────────────────────

/// **N20** : configuration for the cron loop.
#[derive(Debug, Clone)]
pub struct DriftSweepCronConfig {
    /// Time between successive ticks.  Default 24h.
    pub tick_interval: Duration,
    /// Drift threshold ; agents whose last_run_at exceeds this are
    /// re-swept.  Default 23h (allows a small skip-window so
    /// freshly-promoted agents don't double-execute).
    pub drift_threshold: Duration,
}

impl Default for DriftSweepCronConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(24 * 3600),
            drift_threshold: Duration::from_secs(23 * 3600),
        }
    }
}

/// **N20** : the cron loop handle.
///
/// `start` spawns a tokio task that ticks every `tick_interval` ;
/// on each tick, `list_stale` returns the candidates and the driver
/// emits a `tick` callback.  Tests `notify_tick` to fire an early
/// tick deterministically.
pub struct DriftSweepCron {
    pub state: Arc<dyn DriftSweepState>,
    pub config: DriftSweepCronConfig,
    notify: Arc<Notify>,
    ticks_fired: Arc<std::sync::atomic::AtomicU64>,
    stale_seen_total: Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for DriftSweepCron {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriftSweepCron")
            .field("config", &self.config)
            .field(
                "ticks_fired",
                &self.ticks_fired.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}

impl DriftSweepCron {
    pub fn new(
        state: Arc<dyn DriftSweepState>,
        config: DriftSweepCronConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            state,
            config,
            notify: Arc::new(Notify::new()),
            ticks_fired: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            stale_seen_total: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// **N20** : Number of ticks fired since `start`.
    pub fn ticks_fired(&self) -> u64 {
        self.ticks_fired
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// **N20** : Cumulative count of stale entries observed across
    /// all ticks.
    pub fn stale_seen_total(&self) -> u64 {
        self.stale_seen_total
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// **N20 test helper** : trigger a tick immediately (test-only).
    pub fn notify_tick(&self) {
        self.notify.notify_one();
    }

    /// **N20** : Spawn the cron loop.  Returns a `JoinHandle` so
    /// callers can `.abort()` on shutdown.
    ///
    /// On each tick, the loop calls `on_stale_entry` for every agent
    /// that exceeds `drift_threshold`.  Callers wire this to
    /// `GoldGate::run` in production.
    pub fn start<F, Fut>(
        self: Arc<Self>,
        mut on_stale_entry: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: FnMut(DriftSweepStateEntry) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let this = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(this.config.tick_interval);
            // Skip the immediate tick to avoid a burst on startup.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // First tick : interval.tick() resolves immediately the
            // first time ; we consume it to honor the documented
            // semantics (wait one full interval before first work).
            // Tests instead use `notify_tick` to fire early.
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = this.notify.notified() => {}
                }
                this.ticks_fired
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                let now = SystemTime::now();
                let stale = match this
                    .state
                    .list_stale(this.config.drift_threshold, now)
                    .await
                {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                this.stale_seen_total.fetch_add(
                    stale.len() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                for entry in stale {
                    on_stale_entry(entry).await;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn agent(name: &str) -> NanoAgentId {
        NanoAgentId::new(name).unwrap()
    }
    fn version() -> NanoAgentVersion {
        NanoAgentVersion::new("1.0.0").unwrap()
    }

    fn entry(name: &str, last_run: SystemTime) -> DriftSweepStateEntry {
        DriftSweepStateEntry {
            agent_id: agent(name),
            agent_version: version(),
            last_run_at: last_run,
            last_gold_run_id: GoldRunId::new(),
            last_outcome: "passed".into(),
        }
    }

    // ─── InMemoryDriftSweepState ────────────────────────────────

    #[tokio::test]
    async fn upsert_then_lookup() {
        let s = InMemoryDriftSweepState::new();
        let e = entry("a", SystemTime::now());
        s.upsert(e.clone()).await.unwrap();
        let got = s.lookup(&agent("a"), &version()).await.unwrap().unwrap();
        assert_eq!(got.agent_id, e.agent_id);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_entry() {
        let s = InMemoryDriftSweepState::new();
        let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let t2 = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
        s.upsert(entry("a", t1)).await.unwrap();
        s.upsert(entry("a", t2)).await.unwrap();
        let got = s.last_run_at(&agent("a"), &version()).await.unwrap();
        assert_eq!(got, Some(t2));
    }

    #[tokio::test]
    async fn lookup_returns_none_for_missing() {
        let s = InMemoryDriftSweepState::new();
        let got = s.lookup(&agent("missing"), &version()).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn list_stale_filters_by_threshold() {
        let s = InMemoryDriftSweepState::new();
        let now = SystemTime::now();
        s.upsert(entry("fresh", now - Duration::from_secs(60)))
            .await
            .unwrap();
        s.upsert(entry("old", now - Duration::from_secs(86_400)))
            .await
            .unwrap();
        let stale = s
            .list_stale(Duration::from_secs(82_800), now)
            .await
            .unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].agent_id.as_str(), "old");
    }

    #[tokio::test]
    async fn list_stale_deterministic_order() {
        let s = InMemoryDriftSweepState::new();
        let now = SystemTime::now();
        let old = now - Duration::from_secs(86_400);
        for name in ["zebra", "alpha", "mango"] {
            s.upsert(entry(name, old)).await.unwrap();
        }
        let stale = s.list_stale(Duration::from_secs(60), now).await.unwrap();
        assert_eq!(
            stale.iter().map(|e| e.agent_id.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "mango", "zebra"]
        );
    }

    // ─── DriftSweepCron ─────────────────────────────────────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn cron_tick_fires_on_notify() {
        let state = Arc::new(InMemoryDriftSweepState::new());
        let now = SystemTime::now();
        state
            .upsert(entry("old", now - Duration::from_secs(86_400)))
            .await
            .unwrap();

        let cron = DriftSweepCron::new(
            state.clone() as Arc<dyn DriftSweepState>,
            DriftSweepCronConfig {
                tick_interval: Duration::from_secs(86_400),
                drift_threshold: Duration::from_secs(60),
            },
        );

        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_clone = seen.clone();
        let handle = cron.clone().start(move |entry| {
            let s = seen_clone.clone();
            async move {
                s.lock()
                    .unwrap()
                    .push(entry.agent_id.as_str().to_string());
            }
        });

        // Trigger the tick.
        tokio::task::yield_now().await;
        cron.notify_tick();
        // Let the task run.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
        handle.abort();

        let recorded = seen.lock().unwrap();
        assert_eq!(*recorded, vec!["old".to_string()]);
        assert!(cron.ticks_fired() >= 1);
        assert!(cron.stale_seen_total() >= 1);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn cron_skips_fresh_entries() {
        let state = Arc::new(InMemoryDriftSweepState::new());
        let now = SystemTime::now();
        state
            .upsert(entry("fresh", now - Duration::from_secs(60)))
            .await
            .unwrap();

        let cron = DriftSweepCron::new(
            state.clone() as Arc<dyn DriftSweepState>,
            DriftSweepCronConfig {
                tick_interval: Duration::from_secs(86_400),
                drift_threshold: Duration::from_secs(82_800), // 23h
            },
        );

        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cc = counter.clone();
        let handle = cron.clone().start(move |_entry| {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, Ordering::Relaxed);
            }
        });

        tokio::task::yield_now().await;
        cron.notify_tick();
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.abort();

        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn cron_idempotent_across_two_ticks() {
        let state = Arc::new(InMemoryDriftSweepState::new());
        let now = SystemTime::now();
        state
            .upsert(entry("a", now - Duration::from_secs(86_400)))
            .await
            .unwrap();

        let cron = DriftSweepCron::new(
            state.clone() as Arc<dyn DriftSweepState>,
            DriftSweepCronConfig {
                tick_interval: Duration::from_secs(86_400),
                drift_threshold: Duration::from_secs(60),
            },
        );

        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cc = counter.clone();
        let state_for_cb = state.clone();
        let handle = cron.clone().start(move |entry| {
            let cc = cc.clone();
            let s = state_for_cb.clone();
            async move {
                // Simulate the gate updating the drift state ; after
                // this, the agent should no longer be stale.
                cc.fetch_add(1, Ordering::Relaxed);
                let mut fresh = entry.clone();
                fresh.last_run_at = SystemTime::now();
                let _ = s.upsert(fresh).await;
            }
        });

        cron.notify_tick();
        tokio::time::sleep(Duration::from_millis(10)).await;
        cron.notify_tick();
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.abort();

        // Second tick should NOT re-process the now-fresh entry.
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn config_default_is_24h_tick_23h_drift() {
        let cfg = DriftSweepCronConfig::default();
        assert_eq!(cfg.tick_interval, Duration::from_secs(24 * 3_600));
        assert_eq!(cfg.drift_threshold, Duration::from_secs(23 * 3_600));
    }

    #[test]
    fn debug_format_does_not_panic() {
        let state = Arc::new(InMemoryDriftSweepState::new());
        let cron = DriftSweepCron::new(
            state as Arc<dyn DriftSweepState>,
            DriftSweepCronConfig::default(),
        );
        let _ = format!("{cron:?}");
    }
}
