//! RAII handle around an arena borrowed from a [`super::MemoryPool`].
//!
//! On drop, the handle resets its arena and pushes it back to the
//! pool's free-list. A dropped pool (via weak upgrade failure) causes
//! the arena to be released (its backend deallocation runs in its own
//! Drop impl).

use std::sync::atomic::Ordering;
use std::sync::Weak;

use tokio::sync::OwnedSemaphorePermit;

use crate::memory::arena::Arena;
use crate::memory::pool::MemoryPoolInner;

/// RAII handle to a pool-owned arena.
///
/// The underlying arena is accessible via [`Self::arena`] / [`Self::arena_mut`].
/// Dropping the handle returns the arena to the pool after a [`Arena::reset`].
pub struct ArenaHandle {
    arena: Option<Arena>,
    pool: Weak<MemoryPoolInner>,
    /// Live permit keeping the semaphore count down. Released on drop.
    _permit: OwnedSemaphorePermit,
}

impl ArenaHandle {
    pub(crate) fn new(
        arena: Arena,
        pool: Weak<MemoryPoolInner>,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        Self {
            arena: Some(arena),
            pool,
            _permit: permit,
        }
    }

    /// Access the underlying arena.
    ///
    /// # Panics
    ///
    /// Never — the `Option` is only `None` between the start and end
    /// of the `Drop` impl.
    #[must_use]
    pub fn arena(&self) -> &Arena {
        self.arena
            .as_ref()
            .expect("ArenaHandle: arena taken before drop")
    }

    /// Mutable access to the arena (useful to call `reset` explicitly).
    ///
    /// # Panics
    ///
    /// See [`Self::arena`].
    #[must_use]
    pub fn arena_mut(&mut self) -> &mut Arena {
        self.arena
            .as_mut()
            .expect("ArenaHandle: arena taken before drop")
    }

    /// Consume the handle and extract the raw arena.
    ///
    /// The caller becomes responsible for returning the arena to the
    /// pool (or letting it drop). This is an escape hatch; prefer the
    /// automatic RAII path.
    ///
    /// The `arenas_active` and `bytes_active` counters are decremented
    /// at this call so the pool no longer accounts for the extracted
    /// arena. The semaphore permit is released when the handle finishes
    /// dropping (drop of the `_permit` field).
    #[must_use]
    pub fn into_arena(mut self) -> Arena {
        let arena = self.arena.take().expect("ArenaHandle: arena already taken");
        if let Some(pool) = self.pool.upgrade() {
            let bytes = arena.capacity_bytes();
            pool.metrics.arenas_active.fetch_sub(1, Ordering::AcqRel);
            pool.metrics.bytes_active.fetch_sub(bytes, Ordering::AcqRel);
        }
        arena
        // Permit still exists; it will be dropped when `self` drops.
    }
}

impl Drop for ArenaHandle {
    fn drop(&mut self) {
        if let Some(mut arena) = self.arena.take() {
            arena.reset();
            if let Some(pool) = self.pool.upgrade() {
                let bytes = arena.capacity_bytes();
                pool.return_arena(arena);
                pool.metrics.arenas_active.fetch_sub(1, Ordering::AcqRel);
                pool.metrics.bytes_active.fetch_sub(bytes, Ordering::AcqRel);
            }
            // If the pool has been dropped, `arena` is released via its
            // own Drop impl here (backend.deallocate).
        }
    }
}

impl std::fmt::Debug for ArenaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let a = self.arena.as_ref();
        f.debug_struct("ArenaHandle")
            .field("arena", &a)
            .finish_non_exhaustive()
    }
}
