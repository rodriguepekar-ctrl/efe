//! **B3 — Property-based tests for `efe_core::streams`.**
//!
//! Each invariant is exercised against a randomized workload (~100
//! cases per property × varied parameters). The properties verified:
//!
//! - **P1**: `acquire_count == fast + contended` for every workload
//! - **P2**: `active_streams ≤ pool.size()` at every snapshot
//! - **P3**: all metrics are monotonically non-decreasing over time
//! - **P4**: full drain + release returns the pool to its initial state
//!
//! The use of proptest gives us coverage we could never write by hand:
//! random pool sizes, random worker counts, random hold durations,
//! random interleavings.
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::Duration;

use efe_core::streams::{StreamPool, StreamPoolConfig};
use proptest::prelude::*;
use tokio::runtime::Runtime;

/// Run a closure inside a tokio runtime — proptest closures aren't
/// async by default, so we block on a fresh runtime per case. Cheap
/// enough at proptest's default 100 cases.
fn rt_block_on<F: std::future::Future>(f: F) -> F::Output {
    Runtime::new().expect("runtime").block_on(f)
}

// ====================================================================
// P1 — acquire_count == fast + contended
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_acquire_count_equals_fast_plus_contended(
        size in 1usize..=8,
        workers in 1usize..=16,
        iters in 1usize..=4,
    ) {
        rt_block_on(async move {
            let pool = Arc::new(
                StreamPool::new(StreamPoolConfig::default().with_size(size))
                    .expect("pool"),
            );
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                let p = pool.clone();
                handles.push(tokio::spawn(async move {
                    for _ in 0..iters {
                        let h = p.acquire().await.expect("acquire");
                        drop(h);
                    }
                }));
            }
            for h in handles { h.await.expect("join"); }
            let snap = pool.metrics();
            prop_assert_eq!(
                snap.acquire_count,
                snap.fast_acquire_count + snap.contended_acquire_count,
                "P1 invariant broken: acquire_count={} != fast={} + contended={}",
                snap.acquire_count,
                snap.fast_acquire_count,
                snap.contended_acquire_count,
            );
            prop_assert_eq!(
                snap.acquire_count,
                (workers * iters) as u64,
                "expected {} acquires, got {}",
                workers * iters,
                snap.acquire_count,
            );
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// P2 — active_streams ≤ size at every observation point
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_active_streams_bounded_by_size(
        size in 1usize..=8,
        workers in 1usize..=16,
        hold_us in 0u64..200,
    ) {
        rt_block_on(async move {
            let pool = Arc::new(
                StreamPool::new(StreamPoolConfig::default().with_size(size))
                    .expect("pool"),
            );
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                let p = pool.clone();
                handles.push(tokio::spawn(async move {
                    let h = p.acquire().await.expect("acquire");
                    if hold_us > 0 {
                        tokio::time::sleep(Duration::from_micros(hold_us)).await;
                    }
                    let snap = p.metrics();
                    // The CURRENT task holds 1 stream → active ≥ 1.
                    // No other task can hold more than `size` total.
                    let active = snap.active_streams;
                    drop(h);
                    active
                }));
            }
            let mut max_observed = 0u64;
            for h in handles {
                let active = h.await.expect("join");
                if active > max_observed {
                    max_observed = active;
                }
            }
            prop_assert!(
                max_observed <= size as u64,
                "P2 broken: max active_streams observed = {max_observed} > size = {size}"
            );
            // Final cleanup invariant.
            let snap = pool.metrics();
            prop_assert_eq!(
                snap.active_streams, 0,
                "post-cleanup leak: active_streams = {}",
                snap.active_streams
            );
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// P3 — Metrics are monotonic non-decreasing
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_metrics_monotonic_under_random_load(
        size in 1usize..=4,
        ops in 4usize..=32,
    ) {
        rt_block_on(async move {
            let pool = StreamPool::new(StreamPoolConfig::default().with_size(size))
                .expect("pool");
            let mut last_acq = 0u64;
            let mut last_fast = 0u64;
            let mut last_cont = 0u64;
            let mut last_sync = 0u64;
            for _ in 0..ops {
                let h = pool.acquire().await.expect("acquire");
                h.synchronize().await.expect("sync");
                let snap = pool.metrics();
                prop_assert!(snap.acquire_count >= last_acq);
                prop_assert!(snap.fast_acquire_count >= last_fast);
                prop_assert!(snap.contended_acquire_count >= last_cont);
                prop_assert!(snap.synchronize_count >= last_sync);
                last_acq = snap.acquire_count;
                last_fast = snap.fast_acquire_count;
                last_cont = snap.contended_acquire_count;
                last_sync = snap.synchronize_count;
                drop(h);
            }
            Ok(())
        }).unwrap();
    }
}

// ====================================================================
// P4 — Full drain + release returns to initial state
// ====================================================================

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_full_drain_then_release_restores_initial_state(
        size in 1usize..=8,
    ) {
        rt_block_on(async move {
            let pool = StreamPool::new(StreamPoolConfig::default().with_size(size))
                .expect("pool");

            // Drain all slots.
            let mut held = Vec::with_capacity(size);
            for _ in 0..size {
                held.push(pool.acquire().await.expect("acquire"));
            }
            prop_assert_eq!(pool.metrics().active_streams, size as u64);

            // try_acquire should fail now.
            let try_extra = pool.try_acquire();
            prop_assert!(try_extra.is_err(),
                "try_acquire on saturated pool must fail, got Ok");

            // Release everything.
            held.clear();
            prop_assert_eq!(pool.metrics().active_streams, 0);

            // Now we can acquire `size` again.
            let mut held2 = Vec::with_capacity(size);
            for _ in 0..size {
                held2.push(pool.try_acquire().expect("post-drain try_acquire"));
            }
            prop_assert_eq!(pool.metrics().active_streams, size as u64);
            Ok(())
        }).unwrap();
    }
}
