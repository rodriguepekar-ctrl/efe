//! M26 — RAII guard around a `MicrovmSlot` that auto-releases on drop
//! unless the caller explicitly commits the slot to a run.
//!
//! Prevents the M25 orphan-slot leak where any error path between
//! `pool.lease()` and `run::update_status(... Running)` left a slot
//! marked as `Running` in the pool with no `agent_run` row pointing
//! at it.

use crate::sandbox::pool::{MicrovmPool, MicrovmSlot};
use std::sync::Arc;

/// RAII guard. Ownership of the `MicrovmSlot` is held inside ; if the
/// guard is dropped without `commit()`, a background task releases the
/// slot back to the pool.
pub struct MicrovmLease {
    pool: Arc<MicrovmPool>,
    slot: Option<MicrovmSlot>,
    committed: bool,
}

impl MicrovmLease {
    pub fn new(pool: Arc<MicrovmPool>, slot: MicrovmSlot) -> Self {
        Self { pool, slot: Some(slot), committed: false }
    }

    pub fn slot(&self) -> &MicrovmSlot {
        self.slot
            .as_ref()
            .expect("slot must exist before commit")
    }

    /// Commit the lease to the agent run — the guard no longer releases
    /// on drop. Returns the underlying slot to the caller.
    pub fn commit(mut self) -> MicrovmSlot {
        self.committed = true;
        self.slot.take().expect("slot must exist before commit")
    }
}

impl Drop for MicrovmLease {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let Some(slot) = self.slot.take() else {
            return;
        };
        let pool = self.pool.clone();
        let vm_id = slot.handle.vm_id.clone();
        // Release in a background task — Drop is sync.
        tokio::spawn(async move {
            if let Err(e) = pool.release(&vm_id).await {
                tracing::warn!(
                    vm_id = %vm_id,
                    ?e,
                    "MicrovmLease drop : failed to release leaked slot"
                );
            }
            crate::metrics::AGENT_MICROVM_LEAK_RECOVERED_TOTAL.inc();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::firecracker::MockFirecrackerDriver;
    use crate::sandbox::pool::PoolConfig;
    use crate::TenantId;
    use std::time::Duration;

    fn mk_pool() -> Arc<MicrovmPool> {
        Arc::new(MicrovmPool::new(
            TenantId("t1".into()),
            PoolConfig {
                target_size: 2,
                ..Default::default()
            },
            Arc::new(MockFirecrackerDriver::new()),
        ))
    }

    #[tokio::test]
    async fn commit_keeps_slot_running() {
        let pool = mk_pool();
        let slot = pool.lease().await.unwrap();
        let lease = MicrovmLease::new(pool.clone(), slot);
        let _committed = lease.commit();
        // Allow scheduler to process potential drop tasks ; commit
        // means none should fire.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let slots = pool.slots_for_test().await;
        assert_eq!(slots.len(), 1);
        assert!(slots
            .iter()
            .any(|s| s.state == crate::sandbox::pool::SlotState::Running));
    }

    #[tokio::test]
    async fn drop_without_commit_releases() {
        let pool = mk_pool();
        let slot = pool.lease().await.unwrap();
        {
            let _lease = MicrovmLease::new(pool.clone(), slot);
            // Lease drops here without commit.
        }
        // Give the spawned release task time to complete.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let slots = pool.slots_for_test().await;
        // After release, slot moves from Running → Idle.
        assert!(slots
            .iter()
            .any(|s| s.state == crate::sandbox::pool::SlotState::Idle));
    }
}
