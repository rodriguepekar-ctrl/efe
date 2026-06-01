//! No-op metering client.
//!
//! Production default when no billing backend is wired.  All `record_*`
//! calls are silent no-ops.

use async_trait::async_trait;
use efe_nano_sdk::TenantId;

use super::{ExecutionModeKind, MeteringClient};

/// Drops every metering event silently.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMeteringClient;

impl NoopMeteringClient {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MeteringClient for NoopMeteringClient {
    async fn record_call(&self, _tenant: TenantId, _mode: ExecutionModeKind) {}
    async fn record_llm_tokens(&self, _tenant: TenantId, _tokens: u64) {}
    async fn record_vm_seconds(&self, _tenant: TenantId, _seconds: f64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_records_silently() {
        let m = NoopMeteringClient::new();
        m.record_call(TenantId::system(), ExecutionModeKind::ModeC).await;
        m.record_llm_tokens(TenantId::system(), 1000).await;
        m.record_vm_seconds(TenantId::system(), 0.5).await;
        // Should not panic + should not allocate.  Test passes if no panic.
    }
}
