//! **B2 — RAII negative tests for `efe_core::streams::StreamHandle`.**
//!
//! These tests focus on the Drop semantics of [`StreamHandle`]:
//!
//! - Dropping a handle reduces `active_streams` by exactly 1 (S1 invariant).
//! - A handle that **outlives its parent pool** must drop without panic
//!   (defect #6: `Weak::upgrade()` returns `None` is the safe path).
//! - `id()` is stable for the lifetime of the handle.
//! - Acquiring after every release reuses the same set of stream ids
//!   (LIFO for free_list).
//! - `synchronize()` increments the metric exactly once per call.
//!
//! These are the contracts a real CUDA backend (Phase 2) will inherit.
//! Catching a regression here keeps the MVP stub honest.

use std::sync::Arc;
use std::time::Duration;

use efe_core::streams::{StreamPool, StreamPoolConfig};

// ====================================================================
// Drop semantics
// ====================================================================

/// `active_streams` increases on acquire and decreases on drop, by 1.
#[tokio::test]
async fn handle_drop_returns_to_pool() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(2))
        .expect("build pool");
    assert_eq!(pool.metrics().active_streams, 0);

    let h = pool.acquire().await.expect("acquire");
    assert_eq!(pool.metrics().active_streams, 1);

    drop(h);
    assert_eq!(pool.metrics().active_streams, 0);
}

/// Multiple acquires + drops in a row: counter is balanced after each
/// matched pair.
#[tokio::test]
async fn handle_balanced_acquire_drop_sequence() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(4))
        .expect("build pool");

    for cycle in 0..10 {
        let h1 = pool.acquire().await.expect("h1");
        let h2 = pool.acquire().await.expect("h2");
        assert_eq!(
            pool.metrics().active_streams,
            2,
            "cycle {cycle}: 2 active expected"
        );
        drop(h2);
        assert_eq!(pool.metrics().active_streams, 1);
        drop(h1);
        assert_eq!(pool.metrics().active_streams, 0);
    }
}

// ====================================================================
// Pool lifetime — handle outlives pool
// ====================================================================

/// **Defect #6 audit**: when the parent `StreamPool` is dropped before
/// a `StreamHandle`, the handle's drop must NOT panic on the
/// now-broken `Weak::upgrade()`. The handle's RAII contract degrades
/// gracefully: the permit drops normally, but `return_stream` is
/// skipped (the inner is gone anyway).
#[tokio::test]
async fn handle_outlives_pool_no_panic() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(2))
        .expect("build pool");
    let handle = pool.acquire().await.expect("acquire");

    // Drop the pool first. The handle still references the inner via
    // its Arc clone (clone_handle internal), so the inner is kept alive
    // until the handle drops. But once we drop the pool, the LAST
    // strong owner of the inner is the handle itself.
    drop(pool);

    // Drop the handle. With audit-S1-bis fix, the handle must not panic
    // even though the Weak might or might not upgrade depending on the
    // exact ordering. Since the handle holds an OwnedSemaphorePermit
    // referring to the same Arc<Semaphore>, the inner stays alive.
    drop(handle);
    // No assertion needed — the goal is "no panic".
}

// ====================================================================
// Stream id stability
// ====================================================================

/// `handle.id()` returns the same value for the lifetime of the handle.
#[tokio::test]
async fn handle_id_is_stable() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(4))
        .expect("build pool");
    let h = pool.acquire().await.expect("acquire");
    let id1 = h.id();
    tokio::time::sleep(Duration::from_micros(10)).await;
    let id2 = h.id();
    assert_eq!(id1, id2, "id changed during handle lifetime");
}

/// Two simultaneously-held handles have distinct ids.
#[tokio::test]
async fn handle_ids_are_unique_across_concurrent_holders() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(4))
        .expect("build pool");
    let h1 = pool.acquire().await.expect("h1");
    let h2 = pool.acquire().await.expect("h2");
    let h3 = pool.acquire().await.expect("h3");
    assert_ne!(h1.id(), h2.id());
    assert_ne!(h2.id(), h3.id());
    assert_ne!(h1.id(), h3.id());
}

// ====================================================================
// Synchronize semantics
// ====================================================================

/// Each `synchronize()` call increments the metric by exactly 1.
#[tokio::test]
async fn handle_synchronize_increments_metric() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(1))
        .expect("build pool");
    let h = pool.acquire().await.expect("acquire");

    let s0 = pool.metrics().synchronize_count;
    h.synchronize().await.expect("noop sync 1");
    h.synchronize().await.expect("noop sync 2");
    h.synchronize().await.expect("noop sync 3");
    let s1 = pool.metrics().synchronize_count;
    assert_eq!(s1 - s0, 3);
}

// ====================================================================
// LIFO reuse — free_list is a VecDeque, push_back / pop_front,
// so the most recently-released stream is reused last (FIFO).
// ====================================================================

/// Acquire two, release in order, re-acquire two. The two ids must be
/// distinct from each other (S5 sanity), and the cycle must produce a
/// stable set of ids.
#[tokio::test]
async fn handle_free_list_reuses_ids() {
    let pool = StreamPool::new(StreamPoolConfig::default().with_size(2))
        .expect("build pool");
    let (id_a, id_b) = {
        let h_a = pool.acquire().await.expect("a");
        let h_b = pool.acquire().await.expect("b");
        let r = (h_a.id(), h_b.id());
        drop(h_a);
        drop(h_b);
        r
    };
    let (id_a2, id_b2) = {
        let h_a = pool.acquire().await.expect("a2");
        let h_b = pool.acquire().await.expect("b2");
        (h_a.id(), h_b.id())
    };
    // The two ids returned at the second cycle must be the same SET as
    // at the first cycle (the pool only has 2 streams).
    let mut s1 = vec![id_a, id_b];
    s1.sort_unstable();
    let mut s2 = vec![id_a2, id_b2];
    s2.sort_unstable();
    assert_eq!(s1, s2, "free_list lost or invented ids: s1={s1:?}, s2={s2:?}");
}

// ====================================================================
// Arc<StreamPool> + tokio::spawn — the handle is Send + Sync if the
// pool is Send + Sync. This compile-time test is encoded as a runtime
// test by spawning a task that takes ownership of the handle.
// ====================================================================

#[tokio::test]
async fn handle_is_send_can_cross_task_boundaries() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(1))
            .expect("build pool"),
    );
    let p = pool.clone();
    let task = tokio::spawn(async move {
        let h = p.acquire().await.expect("acquire");
        let id = h.id();
        // Drop here — the metric should reflect the release.
        drop(h);
        id
    });
    let id = task.await.expect("join");
    assert!(id < pool.size() as u32);
    assert_eq!(pool.metrics().active_streams, 0);
}
