//! **B1 — Stress tests for `efe_core::streams`.**
//!
//! These tests stress the [`StreamPool`] under high concurrency, sustained
//! cycles, panic-unwinding, and clone-sharing. They ARE the bench
//! pre-flight: if any of them fails, the latency tail of the benchmark
//! is unbounded.
//!
//! Coverage targets (post-audit invariants S1-S5):
//!
//! - **S1**: every acquire has a matching drop (no stream leak)
//! - **S2**: 64×16 concurrent acquires terminate without deadlock
//! - **S3**: `StreamHandle::drop` is panic-safe (catch_unwind)
//! - **S4**: `metrics.active_streams ≤ pool.size()` always
//! - **S5**: `metrics.acquire_count == fast + contended` always
//!
//! Cross-platform note: `tokio::time::sleep(0)` would be a no-op, so we
//! use micro-second sleeps to force task interleavings without burning
//! CI budget.

#![allow(clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::Duration;

use efe_core::streams::{StreamPool, StreamPoolConfig};

// ====================================================================
// S1 — no leak
// ====================================================================

/// 1 000 acquire/release cycles on a pool of size 4. Final
/// `active_streams` must be 0 and `acquire_count` must equal
/// `fast + contended` (S5 invariant double-checked here).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_1000_acquires_no_leak() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(4))
            .expect("build pool"),
    );
    for _ in 0..1000 {
        let h = pool.acquire().await.expect("acquire ok");
        // Hold for a few microseconds to interleave with concurrent workers
        tokio::time::sleep(Duration::from_micros(10)).await;
        drop(h);
    }
    let snap = pool.metrics();
    assert_eq!(
        snap.active_streams, 0,
        "active_streams leaked: expected 0, got {}",
        snap.active_streams
    );
    assert_eq!(
        snap.acquire_count,
        snap.fast_acquire_count + snap.contended_acquire_count,
        "S5 invariant broken: acquire_count={}, fast={}, contended={}",
        snap.acquire_count,
        snap.fast_acquire_count,
        snap.contended_acquire_count,
    );
}

// ====================================================================
// S2 + S4 — concurrent acquires, no deadlock, bounded metrics
// ====================================================================

/// 64 workers × 16 acquires on a pool of size 8. Expected:
///
/// - All workers terminate within the test timeout.
/// - At every periodic sample, `active_streams ≤ size` (S4).
/// - At end, all metrics are coherent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_64_workers_x_16_acquires_under_contention() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(8))
            .expect("build pool"),
    );
    let workers = 64;
    let iters = 16;
    let mut handles = Vec::with_capacity(workers);
    for w in 0..workers {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..iters {
                let h = p.acquire().await.expect("acquire");
                // Hold briefly with worker-specific jitter so workers
                // serialize through the semaphore differently each time.
                tokio::time::sleep(Duration::from_micros(50 + (w as u64) % 7)).await;
                drop(h);
                if i % 4 == 0 {
                    let snap = p.metrics();
                    assert!(
                        snap.active_streams <= 8,
                        "S4 broken: worker={w}, iter={i}: active={} > size=8",
                        snap.active_streams,
                    );
                }
            }
        }));
    }
    for h in handles {
        h.await.expect("worker join");
    }
    let snap = pool.metrics();
    assert_eq!(snap.active_streams, 0, "S1: post-stress leak");
    assert_eq!(
        snap.acquire_count, (workers * iters) as u64,
        "expected {} acquires, got {}",
        workers * iters,
        snap.acquire_count,
    );
    assert_eq!(
        snap.acquire_count,
        snap.fast_acquire_count + snap.contended_acquire_count,
        "S5: arithmetic invariant broken",
    );
}

// ====================================================================
// S5 — metric invariants under random workload
// ====================================================================

/// Mix of fast + slow workers: some hold for 0µs (immediate drop),
/// others hold for 100µs. The sum invariant must hold throughout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_acquire_metric_invariant_under_mixed_workload() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(2))
            .expect("build pool"),
    );
    let mut handles = Vec::new();
    for w in 0..32 {
        let p = pool.clone();
        let hold_us = if w % 2 == 0 { 0 } else { 100 };
        handles.push(tokio::spawn(async move {
            for _ in 0..8 {
                let h = p.acquire().await.expect("acquire");
                if hold_us > 0 {
                    tokio::time::sleep(Duration::from_micros(hold_us)).await;
                }
                drop(h);
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let snap = pool.metrics();
    assert_eq!(
        snap.acquire_count,
        snap.fast_acquire_count + snap.contended_acquire_count,
        "S5 invariant must hold for mixed workloads"
    );
    assert!(
        snap.contended_acquire_count > 0,
        "expected contention with size=2 and 32 workers; got 0"
    );
}

// ====================================================================
// S3 — RAII panic-safety: StreamHandle::drop must release the permit
//      even if the surrounding code unwinds.
// ====================================================================

/// Spawn a task that acquires a handle, then panics. The panic must
/// be caught by the runtime; the handle drop in the task's catch_unwind
/// path must release the permit. Subsequent acquires from a fresh task
/// must succeed (which they would not if the permit had leaked).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_panic_during_hold_releases_permit() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(1))
            .expect("build pool"),
    );

    // Worker that panics after acquire. tokio::spawn already wraps the
    // future in catch_unwind internally — the JoinHandle reports the
    // panic, but the runtime stays healthy.
    let p = pool.clone();
    let handle = tokio::spawn(async move {
        let _h = p.acquire().await.expect("acquire");
        // Panic while holding the handle. _h drops as the stack unwinds.
        panic!("intentional panic to verify RAII");
    });

    // Wait for the panicking task; we expect Err(JoinError).
    let join_result = handle.await;
    assert!(join_result.is_err(), "expected panic propagated as JoinError");

    // After the panic, the pool must be reusable. If the permit had
    // leaked, this acquire would block forever — wrap with timeout.
    let acq = tokio::time::timeout(Duration::from_secs(1), pool.acquire())
        .await
        .expect("S3: drop did not release permit (timeout)")
        .expect("acquire ok");
    drop(acq);

    let snap = pool.metrics();
    assert_eq!(snap.active_streams, 0, "post-recovery leak");
}

// ====================================================================
// Pool clone semantics — multiple StreamPool clones share state
// ====================================================================

/// Cloning a [`StreamPool`] should produce another handle to the SAME
/// underlying pool (Arc-shared). Acquires through clone B must compete
/// for the same slots as clone A.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_pool_clone_shares_state() {
    let pool_a = StreamPool::new(StreamPoolConfig::default().with_size(2))
        .expect("build pool");
    let pool_b = pool_a.clone();
    let pool_c = pool_a.clone();

    // Acquire one from A and one from C → pool saturated.
    let h_a = pool_a.acquire().await.expect("A acquire");
    let h_c = pool_c.acquire().await.expect("C acquire");

    // try_acquire from B must FAIL because the underlying pool is full,
    // even though `pool_b` itself was constructed via Clone.
    let try_b = pool_b.try_acquire();
    assert!(
        try_b.is_err(),
        "clones must share the same semaphore: try_acquire should fail"
    );

    drop(h_a);
    drop(h_c);

    // Now all three clones see size=2 free again.
    let h2 = pool_b.try_acquire().expect("B acquire after release");
    let snap = pool_a.metrics();
    assert_eq!(snap.active_streams, 1);
    drop(h2);
    assert_eq!(pool_a.metrics().active_streams, 0);
}

// ====================================================================
// Edge — large pool size does not OOM nor panic
// ====================================================================

/// **Audit-B8 fix #2 regression**: under heavy stress, no acquired
/// `StreamHandle` should ever observe `id() == u32::MAX` (the
/// previous sentinel value). Post-fix, the impossible state would
/// panic instead — but the panic itself must never fire under
/// well-formed concurrent usage. Run a dense workload and assert
/// every observed id is in `[0, pool.size())`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_no_sentinel_id_ever_returned() {
    let size = 16usize;
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(size))
            .expect("build pool"),
    );
    let workers = 32;
    let iters = 64;

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..iters {
                let h = p.acquire().await.expect("acquire");
                let id = h.id();
                assert!(
                    id < size as u32,
                    "fix #2 regression: id {id} >= size {size} (sentinel?)"
                );
                drop(h);
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
}

/// Build a pool of size 10_000. The MVP backend uses integer
/// pseudo-streams so memory is `~size * 4 bytes` for the free_list.
/// Verify no panic / overflow on either creation or rapid drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_large_pool_size_no_overflow() {
    let pool = Arc::new(
        StreamPool::new(StreamPoolConfig::default().with_size(10_000))
            .expect("build large pool"),
    );
    assert_eq!(pool.size(), 10_000);

    // Drain a small fraction concurrently; we don't drain all 10k
    // because the test goal is sanity, not stress at scale.
    let mut handles = Vec::with_capacity(64);
    for _ in 0..64 {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..32 {
                let h = p.acquire().await.expect("acquire");
                drop(h);
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let snap = pool.metrics();
    assert_eq!(snap.active_streams, 0);
    assert_eq!(snap.acquire_count, 64 * 32);
}
