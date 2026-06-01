//! Contextual information threaded through the genesis pipeline.

use std::time::Instant;

use crate::memory::PoolMetricsSnapshot;
use crate::types::{RequestId, SwarmId};

/// Contextual data threaded through meta → router → cardinality → spawn.
///
/// Allows downstream stages to adapt based on tenant, pool pressure,
/// deadlines, etc.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GenesisContext {
    /// Identifier of the swarm being spawned.
    pub swarm_id: SwarmId,
    /// Tenant (for multi-tenant deployments).
    pub tenant_id: String,
    /// Correlation id from the API layer.
    pub request_id: RequestId,
    /// Optional snapshot of the pool (enables adaptive cardinality in P2+).
    pub pool_metrics: Option<PoolMetricsSnapshot>,
    /// Optional wall-clock deadline propagated from the arbiter.
    pub deadline: Option<Instant>,
}

impl GenesisContext {
    /// Build a minimal context from just the swarm id.
    ///
    /// Other fields default to empty / None.
    #[must_use]
    pub fn minimal(swarm_id: SwarmId) -> Self {
        Self {
            swarm_id,
            tenant_id: String::new(),
            request_id: RequestId::new(),
            pool_metrics: None,
            deadline: None,
        }
    }

    /// Fluent setter for tenant id.
    #[must_use]
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant_id = tenant.into();
        self
    }

    /// Fluent setter for request id.
    #[must_use]
    pub fn with_request_id(mut self, id: RequestId) -> Self {
        self.request_id = id;
        self
    }

    /// Fluent setter for deadline.
    #[must_use]
    pub fn with_deadline(mut self, d: Instant) -> Self {
        self.deadline = Some(d);
        self
    }

    /// Fluent setter for pool metrics.
    #[must_use]
    pub fn with_pool_metrics(mut self, m: PoolMetricsSnapshot) -> Self {
        self.pool_metrics = Some(m);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_initializes_defaults() {
        let id = SwarmId::new();
        let ctx = GenesisContext::minimal(id);
        assert_eq!(ctx.swarm_id, id);
        assert!(ctx.tenant_id.is_empty());
        assert!(ctx.deadline.is_none());
        assert!(ctx.pool_metrics.is_none());
    }

    #[test]
    fn fluent_builders_work() {
        let ctx = GenesisContext::minimal(SwarmId::new())
            .with_tenant("acme-corp")
            .with_deadline(Instant::now());
        assert_eq!(ctx.tenant_id, "acme-corp");
        assert!(ctx.deadline.is_some());
    }
}
