//! Nano-agent batch: SoA container associating a swarm with an arena.
//!
//! In Sprint 0 this was a pure stub. Sprint 1 (M2) enriches it to own
//! an optional [`ArenaHandle`] plus the SoA vectors that describe the
//! swarm's population. Full population of the tensor slices (latent,
//! mask, salience) is done by M3 in Sprint 2.

use crate::memory::handle::ArenaHandle;
use crate::types::{NanoAgentId, SwarmId, TemplateCode};

/// Container for one swarm's nano-agents and their backing HBM.
///
/// Owns the [`ArenaHandle`]: dropping the batch recycles the arena.
pub struct NanoAgentBatch {
    swarm_id: SwarmId,
    count: u32,
    birth_ns: u64,
    arena_handle: Option<ArenaHandle>,
    templates: Vec<TemplateCode>,
    ids: Vec<NanoAgentId>,
    saliences: Vec<f32>,
}

impl NanoAgentBatch {
    /// Construct a batch bound to an arena.
    ///
    /// This is the real constructor used by M3. Sprint 0 code paths
    /// should use [`Self::new_stub`] instead.
    #[must_use]
    pub fn new(
        swarm_id: SwarmId,
        handle: ArenaHandle,
        templates: Vec<TemplateCode>,
        ids: Vec<NanoAgentId>,
    ) -> Self {
        let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        let saliences = vec![1.0; ids.len()];
        Self {
            swarm_id,
            count,
            birth_ns: now_ns(),
            arena_handle: Some(handle),
            templates,
            ids,
            saliences,
        }
    }

    /// Construct a minimal batch without an arena (Sprint 0 compat).
    #[must_use]
    pub fn new_stub(swarm_id: SwarmId, count: u32) -> Self {
        Self {
            swarm_id,
            count,
            birth_ns: now_ns(),
            arena_handle: None,
            templates: Vec::new(),
            ids: (0..count).map(NanoAgentId).collect(),
            saliences: vec![1.0; count as usize],
        }
    }

    /// Swarm this batch belongs to.
    #[must_use]
    pub fn swarm_id(&self) -> SwarmId {
        self.swarm_id
    }

    /// Number of agents.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Age in nanoseconds since spawn.
    #[must_use]
    pub fn age_ns(&self) -> u64 {
        now_ns().saturating_sub(self.birth_ns)
    }

    /// Borrow the template codes (one per agent).
    #[must_use]
    pub fn templates(&self) -> &[TemplateCode] {
        &self.templates
    }

    /// Borrow the per-agent ids.
    #[must_use]
    pub fn ids(&self) -> &[NanoAgentId] {
        &self.ids
    }

    /// Borrow current saliences.
    #[must_use]
    pub fn saliences(&self) -> &[f32] {
        &self.saliences
    }

    /// Mutable access to saliences (used by fusion to update per-round).
    pub fn saliences_mut(&mut self) -> &mut [f32] {
        &mut self.saliences
    }

    /// True iff the batch owns a live arena handle.
    #[must_use]
    pub fn has_arena(&self) -> bool {
        self.arena_handle.is_some()
    }

    /// Access the arena handle (read-only).
    #[must_use]
    pub fn arena_handle(&self) -> Option<&ArenaHandle> {
        self.arena_handle.as_ref()
    }

    /// Consume the batch and reclaim the arena handle.
    ///
    /// Useful in rare paths that need to extend the arena's lifetime
    /// beyond the batch (e.g. handing it over to a successor batch).
    #[must_use]
    pub fn into_arena_handle(mut self) -> Option<ArenaHandle> {
        self.arena_handle.take()
    }
}

impl std::fmt::Debug for NanoAgentBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanoAgentBatch")
            .field("swarm_id", &self.swarm_id)
            .field("count", &self.count)
            .field("age_ns", &self.age_ns())
            .field("has_arena", &self.has_arena())
            .field("n_templates", &self.templates.len())
            .finish_non_exhaustive()
    }
}

fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::backend::{DeviceBackend, HostBackend};
    use crate::memory::config::PoolConfig;
    use crate::memory::pool::MemoryPool;
    use std::sync::Arc;

    #[test]
    fn stub_batch_is_self_consistent() {
        let b = NanoAgentBatch::new_stub(SwarmId::new(), 16);
        assert_eq!(b.count(), 16);
        assert_eq!(b.ids().len(), 16);
        assert_eq!(b.saliences().len(), 16);
        assert!(!b.has_arena());
    }

    #[tokio::test]
    async fn real_batch_owns_arena() {
        let cfg = PoolConfig::default().with_max_arenas(1).with_prealloc(1);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = MemoryPool::new(cfg, backend).expect("pool");
        let handle = pool.acquire(1024, None).await.expect("acquire");
        let batch = NanoAgentBatch::new(
            SwarmId::new(),
            handle,
            vec![TemplateCode::NaSem, TemplateCode::NaLog],
            vec![NanoAgentId(0), NanoAgentId(1), NanoAgentId(2)],
        );
        assert_eq!(batch.count(), 3);
        assert!(batch.has_arena());
    }

    #[tokio::test]
    async fn drop_batch_returns_arena() {
        let cfg = PoolConfig::default().with_max_arenas(2).with_prealloc(0);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = MemoryPool::new(cfg, backend).expect("pool");
        {
            let handle = pool.acquire(1024, None).await.expect("acquire");
            let _batch = NanoAgentBatch::new(SwarmId::new(), handle, vec![], vec![NanoAgentId(0)]);
            assert_eq!(pool.metrics().arenas_active, 1);
        }
        assert_eq!(pool.metrics().arenas_active, 0);
    }

    #[tokio::test]
    async fn into_arena_handle_transfers_ownership() {
        let cfg = PoolConfig::default().with_max_arenas(1).with_prealloc(0);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = MemoryPool::new(cfg, backend).expect("pool");
        let handle = pool.acquire(1024, None).await.expect("acquire");
        let batch = NanoAgentBatch::new(SwarmId::new(), handle, vec![], vec![NanoAgentId(0)]);
        let taken = batch.into_arena_handle().expect("some");
        assert!(taken.arena().capacity_bytes() > 0);
    }

    #[test]
    fn saliences_default_to_one() {
        let b = NanoAgentBatch::new_stub(SwarmId::new(), 4);
        assert!(b.saliences().iter().all(|&s| (s - 1.0).abs() < 1e-6));
    }
}
