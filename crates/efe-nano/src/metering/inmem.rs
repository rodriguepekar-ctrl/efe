//! In-memory metering client for tests.
//!
//! Captures all recorded metering events keyed by `(tenant_id, meter_name)`
//! so tests can assert exact counts.

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::TenantId;
use std::sync::Mutex;

use super::{ExecutionModeKind, MeteringClient};

/// Per-tenant per-meter counters.
///
/// Numeric values are `f64` so `record_vm_seconds(0.05)` doesn't lose
/// precision.  Call counts are also stored as `f64` (always whole).
pub struct InMemoryMeteringClient {
    /// `(tenant, meter_label) → cumulative value`.
    counters: DashMap<(TenantId, &'static str), Mutex<f64>>,
}

impl InMemoryMeteringClient {
    pub fn new() -> Self {
        Self {
            counters: DashMap::new(),
        }
    }

    /// Get the current value of a meter for a tenant (0.0 if never recorded).
    pub fn value(&self, tenant: TenantId, meter_label: &'static str) -> f64 {
        self.counters
            .get(&(tenant, meter_label))
            .map(|m| *m.lock().unwrap())
            .unwrap_or(0.0)
    }

    /// Convenience : value of `nano_agent_calls` for a tenant.
    pub fn nano_agent_calls(&self, tenant: TenantId) -> u64 {
        self.value(tenant, "nano_agent_calls") as u64
    }

    /// Convenience : value of `nano_agent_llm_tokens` for a tenant.
    pub fn nano_agent_llm_tokens(&self, tenant: TenantId) -> u64 {
        self.value(tenant, "nano_agent_llm_tokens") as u64
    }

    /// Convenience : value of `nano_agent_vm_seconds` for a tenant.
    pub fn nano_agent_vm_seconds(&self, tenant: TenantId) -> f64 {
        self.value(tenant, "nano_agent_vm_seconds")
    }

    /// Per-mode call counter (mode_a / mode_b / mode_c labels).
    pub fn calls_for_mode(&self, tenant: TenantId, mode: ExecutionModeKind) -> u64 {
        let label = match mode {
            ExecutionModeKind::ModeA => "nano_agent_calls_mode_a",
            ExecutionModeKind::ModeB => "nano_agent_calls_mode_b",
            ExecutionModeKind::ModeC => "nano_agent_calls_mode_c",
        };
        self.value(tenant, label) as u64
    }

    /// Number of (tenant, meter) entries tracked.
    pub fn entry_count(&self) -> usize {
        self.counters.len()
    }

    /// Reset all counters (test helper).
    pub fn reset(&self) {
        self.counters.clear();
    }

    fn increment(&self, tenant: TenantId, meter_label: &'static str, delta: f64) {
        let entry = self
            .counters
            .entry((tenant, meter_label))
            .or_insert_with(|| Mutex::new(0.0));
        let mut guard = entry.lock().unwrap();
        *guard += delta;
    }
}

impl Default for InMemoryMeteringClient {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InMemoryMeteringClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryMeteringClient")
            .field("entry_count", &self.entry_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl MeteringClient for InMemoryMeteringClient {
    async fn record_call(&self, tenant: TenantId, mode: ExecutionModeKind) {
        // Total + per-mode.
        self.increment(tenant, "nano_agent_calls", 1.0);
        let mode_label = match mode {
            ExecutionModeKind::ModeA => "nano_agent_calls_mode_a",
            ExecutionModeKind::ModeB => "nano_agent_calls_mode_b",
            ExecutionModeKind::ModeC => "nano_agent_calls_mode_c",
        };
        self.increment(tenant, mode_label, 1.0);
    }

    async fn record_llm_tokens(&self, tenant: TenantId, tokens: u64) {
        self.increment(tenant, "nano_agent_llm_tokens", tokens as f64);
    }

    async fn record_vm_seconds(&self, tenant: TenantId, seconds: f64) {
        self.increment(tenant, "nano_agent_vm_seconds", seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn empty_meter_returns_zero() {
        let m = InMemoryMeteringClient::new();
        assert_eq!(m.nano_agent_calls(TenantId::system()), 0);
        assert_eq!(m.nano_agent_llm_tokens(TenantId::system()), 0);
        assert!((m.nano_agent_vm_seconds(TenantId::system()) - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_call_increments_total_and_per_mode() {
        let m = InMemoryMeteringClient::new();
        let t = TenantId::system();
        m.record_call(t, ExecutionModeKind::ModeC).await;
        m.record_call(t, ExecutionModeKind::ModeC).await;
        m.record_call(t, ExecutionModeKind::ModeA).await;
        assert_eq!(m.nano_agent_calls(t), 3);
        assert_eq!(m.calls_for_mode(t, ExecutionModeKind::ModeC), 2);
        assert_eq!(m.calls_for_mode(t, ExecutionModeKind::ModeA), 1);
        assert_eq!(m.calls_for_mode(t, ExecutionModeKind::ModeB), 0);
    }

    #[tokio::test]
    async fn record_llm_tokens_accumulates() {
        let m = InMemoryMeteringClient::new();
        let t = TenantId::system();
        m.record_llm_tokens(t, 100).await;
        m.record_llm_tokens(t, 250).await;
        assert_eq!(m.nano_agent_llm_tokens(t), 350);
    }

    #[tokio::test]
    async fn record_vm_seconds_accumulates_fractional() {
        let m = InMemoryMeteringClient::new();
        let t = TenantId::system();
        m.record_vm_seconds(t, 0.05).await;
        m.record_vm_seconds(t, 0.10).await;
        m.record_vm_seconds(t, 1.5).await;
        let total = m.nano_agent_vm_seconds(t);
        assert!((total - 1.65).abs() < 1e-9, "got {total}");
    }

    #[tokio::test]
    async fn multi_tenant_isolation() {
        let m = InMemoryMeteringClient::new();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        m.record_call(t1, ExecutionModeKind::ModeC).await;
        m.record_call(t2, ExecutionModeKind::ModeC).await;
        m.record_call(t2, ExecutionModeKind::ModeC).await;
        assert_eq!(m.nano_agent_calls(t1), 1);
        assert_eq!(m.nano_agent_calls(t2), 2);
    }

    #[tokio::test]
    async fn reset_clears_state() {
        let m = InMemoryMeteringClient::new();
        let t = TenantId::system();
        m.record_call(t, ExecutionModeKind::ModeC).await;
        assert_eq!(m.nano_agent_calls(t), 1);
        m.reset();
        assert_eq!(m.nano_agent_calls(t), 0);
    }

    #[tokio::test]
    async fn concurrent_records_serialize_per_meter() {
        let m = Arc::new(InMemoryMeteringClient::new());
        let t = TenantId::system();
        let mut handles = Vec::new();
        for _ in 0..100 {
            let m = Arc::clone(&m);
            handles.push(tokio::spawn(async move {
                m.record_call(t, ExecutionModeKind::ModeC).await;
                m.record_llm_tokens(t, 10).await;
                m.record_vm_seconds(t, 0.01).await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(m.nano_agent_calls(t), 100);
        assert_eq!(m.nano_agent_llm_tokens(t), 1000);
        let s = m.nano_agent_vm_seconds(t);
        assert!((s - 1.0).abs() < 1e-9, "got {s}");
    }

    #[tokio::test]
    async fn entry_count_tracks_unique_meters() {
        let m = InMemoryMeteringClient::new();
        let t = TenantId::system();
        assert_eq!(m.entry_count(), 0);
        m.record_call(t, ExecutionModeKind::ModeC).await;
        // Increments 2 entries : nano_agent_calls + nano_agent_calls_mode_c
        assert_eq!(m.entry_count(), 2);
        m.record_llm_tokens(t, 10).await;
        assert_eq!(m.entry_count(), 3);
    }
}
