//! Integration tests covering the full M2 surface: pool + arena +
//! handle + backend, exercised end-to-end without the rest of EFE.

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_core::memory::{
    copy_slice_to_host, DeviceBackend, DeviceSlice, HostBackend, MemoryPool, PoolConfig,
};
use efe_core::EfeError;

fn make_pool(max: usize, prealloc: usize, arena_bytes: u64) -> MemoryPool {
    let cfg = PoolConfig::default()
        .with_max_arenas(max)
        .with_prealloc(prealloc)
        .with_arena_size(arena_bytes);
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MemoryPool::new(cfg, backend).expect("pool")
}

// ---------- Soak / leak ----------

#[tokio::test]
async fn soak_10k_cycles_no_leak() {
    let pool = make_pool(4, 2, 1 << 14);
    for _ in 0..10_000 {
        let h = pool.acquire(1024, None).await.expect("acquire");
        let _slice = h.arena().alloc_zeroed::<u32>(128).expect("alloc");
    }
    let m = pool.metrics();
    assert_eq!(m.arenas_active, 0);
    assert_eq!(m.bytes_active, 0);
}

#[tokio::test]
async fn drop_handle_releases_backend_memory() {
    // Inspect the backend's allocation count to confirm HBM is freed
    // when the pool shuts down.
    let backend = Arc::new(HostBackend::new());
    {
        let cfg = PoolConfig::default()
            .with_max_arenas(4)
            .with_prealloc(2)
            .with_arena_size(1 << 13);
        let pool = MemoryPool::new(cfg, backend.clone() as Arc<dyn DeviceBackend>).expect("pool");
        assert_eq!(backend.live_allocations(), 2);
        {
            let h = pool.acquire(128, None).await.expect("a");
            let _s = h.arena().alloc_zeroed::<u8>(64).unwrap();
            // Still 2 arenas, one is in use by the handle.
            assert_eq!(backend.live_allocations(), 2);
        }
        // Handle dropped, arena returned. Still 2 arenas cached.
        assert_eq!(backend.live_allocations(), 2);
    }
    // Pool dropped: all arenas released.
    assert_eq!(backend.live_allocations(), 0);
}

// ---------- Concurrency ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_acquire_respects_saturation_limit() {
    let pool = Arc::new(make_pool(3, 0, 1 << 13));
    let barrier = Arc::new(tokio::sync::Barrier::new(10));
    let mut handles = vec![];
    for _ in 0..10 {
        let p = pool.clone();
        let b = barrier.clone();
        handles.push(tokio::spawn(async move {
            b.wait().await;
            let h = p.acquire(128, None).await.expect("ok");
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(h);
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let m = pool.metrics();
    // No more than 3 arenas were ever allocated (saturation respected).
    assert!(m.arenas_allocated_total <= 3);
    assert_eq!(m.arenas_active, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_alloc_inside_single_arena() {
    let pool = make_pool(1, 1, 1 << 16);
    let handle = pool.acquire(0, None).await.expect("a");
    let arena = handle.arena();

    // `std::thread::scope` guarantees that every spawned thread is
    // joined before the scope ends, so borrowing `arena` across the
    // threads is sound without any `unsafe`.
    let all: Vec<usize> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(8);
        for _ in 0..8 {
            handles.push(s.spawn(|| {
                let mut ptrs = Vec::with_capacity(50);
                for _ in 0..50 {
                    if let Ok(slice) = arena.alloc_zeroed::<u8>(64) {
                        // SAFETY: debug check against the live generation.
                        let ptr = unsafe { slice.as_device_ptr(arena.generation()) };
                        ptrs.push(ptr.as_ptr() as usize);
                    }
                }
                ptrs
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("join"))
            .collect()
    });

    let mut sorted = all.clone();
    sorted.sort_unstable();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        before,
        "overlapping allocations detected: {} duplicates",
        before - sorted.len()
    );
}

// ---------- Determinism ----------

#[tokio::test]
async fn alloc_is_always_zero_filled() {
    let pool = make_pool(1, 0, 1 << 14);
    let handle = pool.acquire(1024, None).await.expect("a");
    let arena = handle.arena();

    // First, poison the backend memory via a raw write.
    let slice: DeviceSlice<'_, u8> = arena.alloc_zeroed(256).expect("alloc");
    // SAFETY: host backend exposes a dereferenceable pointer; we write
    // garbage to verify the subsequent alloc memsets it.
    unsafe {
        let p = slice.as_device_ptr(arena.generation()).as_ptr();
        std::ptr::write_bytes(p, 0xAB, slice.len());
    }

    // Second alloc must still be zero-filled (fresh region).
    let next: DeviceSlice<'_, u32> = arena.alloc_zeroed(64).expect("next");
    let bytes = copy_slice_to_host(arena, &next).expect("copy");
    assert!(bytes.iter().all(|&b| b == 0), "second alloc not zeroed");
}

#[tokio::test]
async fn reset_invalidates_generation() {
    let pool = make_pool(1, 1, 1 << 13);
    let handle = pool.acquire(128, None).await.expect("a");
    let gen_initial = handle.arena().generation();
    drop(handle); // reset + return to pool
    let handle2 = pool.acquire(128, None).await.expect("a2");
    let gen_after = handle2.arena().generation();
    assert!(
        gen_after > gen_initial,
        "generation should increase after reset"
    );
}

// ---------- Error paths ----------

#[tokio::test]
async fn oversize_request_forces_miss_and_tailored_arena() {
    let pool = make_pool(2, 0, 1 << 13);
    let h = pool.acquire(1 << 20, None).await.expect("oversized");
    assert!(h.arena().capacity_bytes() >= 1 << 20);
    assert_eq!(pool.metrics().miss_count, 1);
}

#[tokio::test]
async fn deadline_past_returns_budget_exhausted() {
    let pool = make_pool(1, 0, 1 << 13);
    let _h = pool.acquire(128, None).await.expect("first");
    let past = Instant::now()
        .checked_sub(Duration::from_millis(10))
        .unwrap();
    let err = pool.acquire(128, Some(past)).await.unwrap_err();
    assert!(matches!(
        err,
        EfeError::BudgetExhausted { kind: "wall_clock" }
    ));
}

#[tokio::test]
async fn alloc_beyond_arena_capacity_returns_hbm_alloc_failed() {
    let pool = make_pool(1, 1, 4096);
    let h = pool.acquire(1024, None).await.expect("a");
    let _ok = h.arena().alloc_zeroed::<u8>(2048).expect("ok");
    let err = h.arena().alloc_zeroed::<u8>(4096).unwrap_err();
    assert!(matches!(err, EfeError::HbmAllocFailed { .. }));
}

// ---------- Metrics ----------

#[tokio::test]
async fn metrics_snapshot_reflects_activity() {
    let pool = make_pool(2, 1, 1 << 13);
    let before = pool.metrics();
    assert_eq!(before.arenas_active, 0);

    let h = pool.acquire(1024, None).await.expect("a");
    let m = pool.metrics();
    assert_eq!(m.arenas_active, 1);
    assert_eq!(m.acquire_count, 1);
    assert_eq!(m.hit_count, 1);

    drop(h);
    let after = pool.metrics();
    assert_eq!(after.arenas_active, 0);
}

#[tokio::test]
async fn hit_ratio_trends_up() {
    let pool = make_pool(4, 0, 1 << 13);

    // 2 cold misses.
    {
        let _h1 = pool.acquire(128, None).await.expect("a");
        let _h2 = pool.acquire(128, None).await.expect("a");
    }

    // 4 hot hits (from free-list).
    for _ in 0..4 {
        let _h = pool.acquire(128, None).await.expect("a");
    }

    let m = pool.metrics();
    assert_eq!(m.hit_count, 4);
    assert_eq!(m.miss_count, 2);
    assert!((m.hit_ratio - 4.0 / 6.0).abs() < 1e-6);
}
