// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G7 + I9** : thread-local deterministic seed
//! forwarding.
//!
//! The N21 `SwarmAgentContext` carries an `rng_seed: u64` that the
//! orchestrator derives deterministically from `(swarm_id,
//! agent_id)`.  Stateless agents ignore it ; agents that need
//! random tie-breaking, sampling, or randomised LSH MUST honour
//! it for replay (I13).
//!
//! The adapter writes the seed to a thread-local cell **before**
//! dispatching the wrapped runner and clears it afterwards via a
//! RAII guard.  Runners read it through [`current_seed`].
//!
//! ## Why thread-local and not an argument
//!
//! The `Runtime` / `NanoRunner` traits do not take a seed.  We
//! could thread one through every signature but it would pollute
//! the entire SDK with a parameter that 95 % of agents ignore.
//! The thread-local stays scoped to the dispatch ; cross-task
//! work spawned by the runner sees the seed only if the runner
//! forwards it explicitly to the spawned tasks.

use std::cell::Cell;

thread_local! {
    static SEED: Cell<Option<u64>> = const { Cell::new(None) };
}

/// **N23** : install `seed` as the current thread-local seed.
///
/// Used internally by [`SeedGuard`].  Public for advanced uses
/// (e.g., a runner manually opting child tasks into the seed).
pub fn set_seed(seed: u64) {
    SEED.with(|c| c.set(Some(seed)));
}

/// **N23** : clear the current thread-local seed.
pub fn clear_seed() {
    SEED.with(|c| c.set(None));
}

/// **N23** : read the current thread-local seed if installed.
///
/// Returns `None` outside a [`SeedGuard`] scope.
pub fn current_seed() -> Option<u64> {
    SEED.with(|c| c.get())
}

/// **N23** : RAII guard that installs a seed for its lifetime and
/// restores the prior seed (often `None`) on drop.
///
/// Use exactly one `SeedGuard` per dispatch :
///
/// ```
/// use efe_nano_swarm_adapter::{current_seed, SeedGuard};
/// {
///     let _g = SeedGuard::new(42);
///     assert_eq!(current_seed(), Some(42));
/// }
/// assert!(current_seed().is_none());
/// ```
pub struct SeedGuard {
    prev: Option<u64>,
}

impl SeedGuard {
    /// **N23** : install `seed` as the current thread-local seed.
    /// Records the prior value so [`Drop`] restores it.
    pub fn new(seed: u64) -> Self {
        let prev = current_seed();
        set_seed(seed);
        Self { prev }
    }
}

impl Drop for SeedGuard {
    fn drop(&mut self) {
        SEED.with(|c| c.set(self.prev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_seed_outside_guard() {
        clear_seed();
        assert!(current_seed().is_none());
    }

    #[test]
    fn guard_installs_and_clears() {
        clear_seed();
        {
            let _g = SeedGuard::new(7);
            assert_eq!(current_seed(), Some(7));
        }
        assert!(current_seed().is_none());
    }

    #[test]
    fn nested_guards_restore_inner_then_outer() {
        clear_seed();
        {
            let _outer = SeedGuard::new(1);
            assert_eq!(current_seed(), Some(1));
            {
                let _inner = SeedGuard::new(2);
                assert_eq!(current_seed(), Some(2));
            }
            assert_eq!(current_seed(), Some(1));
        }
        assert!(current_seed().is_none());
    }

    #[test]
    fn clear_seed_works() {
        set_seed(42);
        assert_eq!(current_seed(), Some(42));
        clear_seed();
        assert!(current_seed().is_none());
    }

    #[test]
    fn set_seed_overrides_existing() {
        set_seed(1);
        set_seed(2);
        assert_eq!(current_seed(), Some(2));
        clear_seed();
    }

    #[test]
    fn seed_isolated_across_threads() {
        set_seed(99);
        let h = std::thread::spawn(current_seed);
        let other = h.join().unwrap();
        // The new thread sees None — thread-locality.
        assert!(other.is_none());
        // Our thread still sees its seed.
        assert_eq!(current_seed(), Some(99));
        clear_seed();
    }

    #[tokio::test]
    async fn guard_drop_runs_at_scope_end_async() {
        clear_seed();
        async {
            let _g = SeedGuard::new(123);
            assert_eq!(current_seed(), Some(123));
        }
        .await;
        // Note : thread-locals + tokio multi-thread can land on
        // a different worker for the assertion below ; we only
        // assert that no seed leaks on the current thread.
        assert!(current_seed().is_none());
    }
}
