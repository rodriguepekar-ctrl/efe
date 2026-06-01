//! **Sprint D2 — Memory pool / arena realism anti-regression tests.**
//!
//! Run as part of the regular `cargo test -p efe-core` suite. Their
//! purpose is to **prove the D2 bench is measuring the real
//! infrastructure**, not a stripped-down or degenerate version of it.
//!
//! Mirror of `crates/efe-server/tests/bench_realism_check.rs` (D1):
//! every claim the bench makes is locked in by a test that fails
//! BEFORE the bench can produce misleading numbers.
//!
//! The 4 tests cover :
//!
//! 1. **`pool_acquire_does_not_leak`** — 1 000 acquire/release
//!    cycles leave the pool in a clean state (no live arena, no
//!    leaked bytes).
//!
//! 2. **`arena_alloc_pattern_matches_expected_cursor`** — the
//!    realistic Mvp pattern (latents + mask + attention +
//!    confidence + scratch on the small fixture) advances the
//!    cursor by exactly 8 960 bytes. This pins down the
//!    `TENSOR_ALIGNMENT = 256` invariant : if it ever changes, the
//!    bench's SLO targets need re-calibration and this test trips.
//!
//! 3. **`pool_saturation_respects_max_arenas`** — 8 concurrent
//!    tasks racing for 2 slots never cause more than 2 fresh
//!    allocations (the pool recycles).
//!
//! 4. **`host_backend_no_dangling_after_pool_drop`** — destroying
//!    a pool releases every arena it ever held to the backend.
//!
//! # Why duplicate `make_pool` here
//!
//! The bench helper `crates/efe-core/benches/common/mod.rs::make_pool`
//! lives in a *bench* compilation unit; integration tests under
//! `tests/` are a separate crate and cannot import from `benches/`.
//! Rather than expose the helper as `pub mod bench_helpers` in the
//! library (which would leak bench-only plumbing into the public
//! API), we inline the ~6-line factory here. The duplication is
//! negligible and keeps the public surface clean.

use std::sync::Arc;
use std::time::Duration;

use efe_core::memory::{
    Arena, DeviceBackend, HostBackend, MemoryPool, PoolConfig, MIN_ARENA_BYTES,
    TENSOR_ALIGNMENT,
};

const TEST_ARENA_SIZE: u64 = 64 * 1024; // 64 KiB — large enough for the
                                         // small fixture (8 KiB used)
                                         // with comfortable margin.

fn make_pool(prealloc: usize, max: usize, arena_size: u64) -> MemoryPool {
    let cfg = PoolConfig::default()
        .with_prealloc(prealloc)
        .with_max_arenas(max)
        .with_arena_size(arena_size);
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MemoryPool::new(cfg, backend).expect("pool")
}

// =====================================================================
// Test 1 — Pool acquire/release loop does not leak
// =====================================================================
//
// Drives 1 000 acquire/drop cycles through a 2-prealloc / 4-max pool.
// At the end :
//
// * `arenas_active == 0` — every handle drop returned its arena.
// * `bytes_active == 0` — bytes counter is symmetric.
// * `arenas_allocated_total <= 4` — the pool never grew beyond
//   `max_arenas` despite 1 000 cycles (free-list recycles).
// * `hit_count + miss_count >= 1 000` — every iteration was
//   accounted for.
//
// Detects : a regression in `MemoryPoolInner::return_arena` (e.g. a
// missed `metrics.arenas_active.fetch_sub`), a leaked
// `ArenaHandle::Drop` impl, an unbounded growth of the free-list.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_acquire_does_not_leak() {
    let pool = make_pool(2, 4, TEST_ARENA_SIZE);
    for i in 0..1_000 {
        let h = pool
            .acquire(1024, None)
            .await
            .unwrap_or_else(|e| panic!("acquire #{i} failed: {e}"));
        // drop(h) at scope end → return to free-list.
        let _ = h;
    }
    let m = pool.metrics();
    assert_eq!(
        m.arenas_active, 0,
        "{} arenas still active after 1 000 cycles (handle drop regressed?)",
        m.arenas_active
    );
    assert_eq!(
        m.bytes_active, 0,
        "{} bytes still active (asymmetric metrics?)",
        m.bytes_active
    );
    assert!(
        m.arenas_allocated_total <= 4,
        "pool grew to {} arenas (free-list not recycling?)",
        m.arenas_allocated_total
    );
    assert!(
        m.hit_count + m.miss_count >= 1_000,
        "metrics undercount: hit={} + miss={} < 1 000",
        m.hit_count,
        m.miss_count
    );
}

// =====================================================================
// Test 2 — Arena alloc pattern matches expected cursor (small fixture)
// =====================================================================
//
// Reproduces the Mvp swarm pattern on `dim=128, n_agents=8` and
// asserts the post-sequence cursor matches the value computed by
// hand from `align_up(size, TENSOR_ALIGNMENT)`.
//
// Calculation (with `TENSOR_ALIGNMENT = 256`) :
//
// | step | type | count | raw bytes | aligned | running cursor |
// |------|------|------:|----------:|--------:|---------------:|
// | latents    | f32 | 1 024 | 4 096 | 4 096 | 4 096 |
// | mask       | u8  |     8 |     8 |   256 | 4 352 |
// | attention  | f32 |    64 |   256 |   256 | 4 608 |
// | confidence | f32 |     8 |    32 |   256 | 4 864 |
// | scratch    | f32 | 1 024 | 4 096 | 4 096 | 8 960 |
//
// Detects : a regression in `align_up`, in the `TENSOR_ALIGNMENT`
// constant, in the `next_multiple_of` math, or in any future change
// that removes alignment padding for sub-256B types.

#[test]
fn arena_alloc_pattern_matches_expected_cursor_small() {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let arena = Arena::new(TEST_ARENA_SIZE, backend, 0).expect("arena");

    const DIM: usize = 128;
    const N_AGENTS: usize = 8;

    // Step 1 : latents
    let _l = arena
        .alloc_zeroed::<f32>(N_AGENTS * DIM)
        .expect("latents");
    assert_eq!(
        arena.cursor_bytes(),
        4_096,
        "after latents: 8×128×4 = 4 096 (already 256-aligned)"
    );

    // Step 2 : mask
    let _m = arena.alloc_zeroed::<u8>(N_AGENTS).expect("mask");
    assert_eq!(
        arena.cursor_bytes(),
        4_352,
        "after mask: 4 096 + align_up(8, 256) = 4 352"
    );

    // Step 3 : attention
    let _a = arena
        .alloc_zeroed::<f32>(N_AGENTS * N_AGENTS)
        .expect("attention");
    assert_eq!(
        arena.cursor_bytes(),
        4_608,
        "after attention: 4 352 + align_up(256, 256) = 4 608"
    );

    // Step 4 : confidence
    let _c = arena.alloc_zeroed::<f32>(N_AGENTS).expect("confidence");
    assert_eq!(
        arena.cursor_bytes(),
        4_864,
        "after confidence: 4 608 + align_up(32, 256) = 4 864"
    );

    // Step 5 : scratch
    let _s = arena
        .alloc_zeroed::<f32>(N_AGENTS * DIM)
        .expect("scratch");
    assert_eq!(
        arena.cursor_bytes(),
        8_960,
        "after scratch: 4 864 + 4 096 = 8 960 (the SLO-D2-8 footprint)"
    );

    // Belt-and-suspenders: TENSOR_ALIGNMENT pinned at 256.
    assert_eq!(
        TENSOR_ALIGNMENT, 256,
        "TENSOR_ALIGNMENT changed — bench SLO targets need recalibration"
    );
    // And MIN_ARENA_BYTES still 4 KiB (otherwise the bench's
    // `arena_size = next_multiple_of(256).max(MIN)` math is off).
    assert_eq!(
        MIN_ARENA_BYTES, 4 * 1024,
        "MIN_ARENA_BYTES changed — recheck bench arena_size sizing"
    );
}

// =====================================================================
// Test 3 — Saturation respects max_arenas
// =====================================================================
//
// 8 concurrent tasks acquire+sleep+release on a pool with 2 slots
// (no prealloc). The semaphore guarantees at most 2 acquires can
// run concurrently; the pool never allocates more than 2 fresh
// arenas because the released ones cycle through the free-list.
//
// Detects : a regression in the `tokio::Semaphore` permit
// management, a missed `inner.return_arena` in
// `ArenaHandle::Drop`, an off-by-one in `max_arenas` enforcement.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pool_saturation_respects_max_arenas() {
    let pool = make_pool(0, 2, TEST_ARENA_SIZE);
    let pool = Arc::new(pool);

    let mut tasks = Vec::with_capacity(8);
    for i in 0..8 {
        let p = pool.clone();
        tasks.push(tokio::spawn(async move {
            let h = p
                .acquire(512, None)
                .await
                .unwrap_or_else(|e| panic!("task #{i}: {e}"));
            // Sleep long enough that the semaphore wait queue
            // builds up — otherwise tasks may serialise themselves
            // through fast acquire/release without contention.
            tokio::time::sleep(Duration::from_millis(5)).await;
            drop(h);
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }
    let m = pool.metrics();
    assert!(
        m.arenas_allocated_total <= 2,
        "pool allocated {} arenas (semaphore cap leak?)",
        m.arenas_allocated_total
    );
    assert_eq!(
        m.arenas_active, 0,
        "{} arenas still held after all tasks finished",
        m.arenas_active
    );
}

// =====================================================================
// Test 4 — Pool drop releases all backend allocations
// =====================================================================
//
// Build a pool with `prealloc=2`, observe `live_allocations == 2`,
// drop the pool, observe `live_allocations == 0`. The external
// `Arc<HostBackend>` reference keeps the backend alive across the
// pool drop so we can assert.
//
// Detects : a future regression where `Drop for Arena` forgets to
// call `backend.deallocate`, or where the pool short-circuits the
// drop path on shutdown.

#[test]
fn host_backend_no_dangling_after_pool_drop() {
    let backend = Arc::new(HostBackend::new());
    let backend_dyn: Arc<dyn DeviceBackend> = backend.clone();

    {
        let cfg = PoolConfig::default()
            .with_max_arenas(4)
            .with_prealloc(2)
            .with_arena_size(TEST_ARENA_SIZE);
        let pool = MemoryPool::new(cfg, backend_dyn).expect("pool");
        assert_eq!(
            backend.live_allocations(),
            2,
            "prealloc=2 should give 2 live backend allocations"
        );
        // Acquire one extra arena to grow the pool to 3 live.
        let h = pool
            .try_acquire(1024)
            .expect("try_acquire on warm pool")
            ;
        // 3rd alloc came from free-list (one of the 2 prealloc),
        // so live_allocations stays at 2 (no fresh alloc) — but
        // bytes_active should be > 0.
        assert!(pool.metrics().bytes_active >= 1024);
        drop(h);

        // Force a fresh alloc by acquiring beyond the prealloc.
        // Pool has prealloc=2, max=4; we can hold 4 simultaneously.
        let _h1 = pool.try_acquire(1024).expect("h1");
        let _h2 = pool.try_acquire(1024).expect("h2");
        let _h3 = pool.try_acquire(1024).expect("h3");
        let _h4 = pool.try_acquire(1024).expect("h4");
        // Now live_allocations >= 4 (we forced 2 fresh allocs
        // beyond the prealloc).
        assert!(
            backend.live_allocations() >= 4,
            "expected >= 4 live, got {}",
            backend.live_allocations()
        );
        // Drop everything, then drop the pool.
        // (h1..h4 drop at scope end below.)
    }

    // Pool, all handles, and all arenas have dropped here.
    assert_eq!(
        backend.live_allocations(),
        0,
        "{} dangling allocations after pool drop",
        backend.live_allocations()
    );
}
