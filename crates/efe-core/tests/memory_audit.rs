//! Security / soundness regression tests for M2.
//!
//! Each test encodes one of the audit findings fixed in the second
//! review pass. Any regression on these invariants must block the PR.

use std::sync::Arc;

use efe_core::memory::{
    Arena, DeviceBackend, DeviceSlice, HostBackend, MemoryPool, PoolConfig, RawAllocation,
};
use efe_core::EfeError;

fn make_pool(max: usize, prealloc: usize, size: u64) -> MemoryPool {
    let cfg = PoolConfig::default()
        .with_max_arenas(max)
        .with_prealloc(prealloc)
        .with_arena_size(size);
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    MemoryPool::new(cfg, backend).expect("pool")
}

// =====================================================================
// F1 - into_arena must decrement active counters
// =====================================================================

#[tokio::test]
async fn into_arena_decrements_metrics() {
    let pool = make_pool(2, 0, 1 << 13);
    let handle = pool.acquire(1024, None).await.expect("acquire");
    assert_eq!(pool.metrics().arenas_active, 1);

    let capacity = handle.arena().capacity_bytes();
    let arena = handle.into_arena();

    let metrics = pool.metrics();
    assert_eq!(
        metrics.arenas_active, 0,
        "into_arena must decrement arenas_active"
    );
    assert_eq!(
        metrics.bytes_active, 0,
        "into_arena must decrement bytes_active"
    );
    assert_eq!(arena.capacity_bytes(), capacity);
    // arena drops here — backend deallocates automatically.
}

#[tokio::test]
async fn into_arena_releases_semaphore_slot() {
    let pool = make_pool(1, 0, 1 << 13);
    let handle = pool.acquire(1024, None).await.expect("a");
    let _extracted = handle.into_arena();
    // Even though the arena is not returned to the pool, the slot
    // should be free (permit released when handle drops).
    let handle2 = pool.try_acquire(1024).expect("slot free after into_arena");
    assert!(handle2.arena().capacity_bytes() >= 1024);
}

// =====================================================================
// F2 - alloc_zeroed must reject size overflows
// =====================================================================

#[tokio::test]
async fn alloc_zeroed_rejects_count_size_overflow() {
    let pool = make_pool(1, 1, 1 << 14);
    let handle = pool.acquire(128, None).await.expect("a");
    let arena = handle.arena();

    // count * size_of::<u64>() overflows usize on any 64-bit host.
    let huge = usize::MAX / 4 + 1;
    let err = arena.alloc_zeroed::<u64>(huge).unwrap_err();
    assert!(
        matches!(err, EfeError::HbmAllocFailed { .. }),
        "overflow must produce HbmAllocFailed, got {err:?}"
    );
}

#[tokio::test]
async fn alloc_zeroed_rejects_request_above_arena_capacity() {
    let pool = make_pool(1, 1, 4096);
    let handle = pool.acquire(1024, None).await.expect("a");
    let arena = handle.arena();

    // Request bigger than arena capacity — must fail before bumping
    // the cursor (so future small allocations still work).
    let err = arena.alloc_zeroed::<u8>(8192).unwrap_err();
    assert!(matches!(err, EfeError::HbmAllocFailed { .. }));
    // A subsequent reasonable allocation still succeeds.
    let _small = arena.alloc_zeroed::<u8>(64).expect("still ok");
}

// =====================================================================
// F3 - cursor overflow protection (fails closed, never corrupts)
// =====================================================================

#[tokio::test]
async fn cursor_near_capacity_still_rejects_extra_alloc() {
    let pool = make_pool(1, 1, 4096);
    let handle = pool.acquire(1024, None).await.expect("a");
    let arena = handle.arena();
    // Fill the arena.
    let _big = arena.alloc_zeroed::<u8>(4096).expect("fill");
    // Now cursor == 4096. Any further alloc must fail, not wrap.
    let err = arena.alloc_zeroed::<u8>(1).unwrap_err();
    assert!(matches!(err, EfeError::HbmAllocFailed { .. }));
    let err2 = arena.alloc_zeroed::<u8>(usize::MAX / 2).unwrap_err();
    assert!(matches!(err2, EfeError::HbmAllocFailed { .. }));
}

// =====================================================================
// F4 - RawAllocation::validate_against must catch stale pointers
// =====================================================================

#[tokio::test]
async fn raw_allocation_validate_detects_reset() {
    let pool = make_pool(1, 1, 1 << 13);
    let mut handle = pool.acquire(128, None).await.expect("a");
    let raw: RawAllocation = handle.arena().alloc_zeroed_raw(256).expect("raw");

    // Initially valid.
    raw.validate_against(handle.arena()).expect("valid at t0");

    // Reset the arena explicitly via arena_mut.
    handle.arena_mut().reset();

    // Now the raw allocation is stale — must be detected.
    let err = raw.validate_against(handle.arena()).unwrap_err();
    assert!(matches!(err, EfeError::Internal(_)));
}

#[tokio::test]
async fn raw_allocation_validate_ok_when_still_live() {
    let pool = make_pool(1, 1, 1 << 13);
    let handle = pool.acquire(128, None).await.expect("a");
    let raw = handle.arena().alloc_zeroed_raw(512).expect("raw");
    // Multiple valid checks across other allocs — no reset happened.
    let _other = handle.arena().alloc_zeroed::<u8>(128).expect("other");
    raw.validate_against(handle.arena()).expect("still valid");
}

// =====================================================================
// F5 - bonus: concurrent alloc uniqueness with thread::scope
// =====================================================================

#[test]
fn arena_concurrent_alloc_produces_unique_pointers() {
    // This is a pure-Rust (no tokio) variant that exercises the scope-
    // based pattern directly on a non-pooled Arena. Fills the role of
    // verifying the borrow-based concurrent allocation path.
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let arena = Arena::new(1 << 16, backend, 0).expect("arena");

    let all: Vec<usize> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(8);
        for _ in 0..8 {
            handles.push(s.spawn(|| {
                let mut ptrs = Vec::with_capacity(100);
                for _ in 0..100 {
                    if let Ok(slice) = arena.alloc_zeroed::<u8>(32) {
                        let p = unsafe { slice.as_device_ptr(arena.generation()) };
                        ptrs.push(p.as_ptr() as usize);
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
    assert_eq!(sorted.len(), before, "duplicate pointers detected");
}

// =====================================================================
// Additional invariants
// =====================================================================

#[tokio::test]
async fn release_does_not_underflow_metrics_after_many_cycles() {
    let pool = make_pool(2, 0, 1 << 13);
    for _ in 0..100 {
        let h = pool.acquire(64, None).await.expect("a");
        let _ = h.arena().alloc_zeroed::<u32>(8).unwrap();
    }
    let m = pool.metrics();
    // bytes_active must return to 0, not wrap around.
    assert_eq!(m.bytes_active, 0);
    assert_eq!(m.arenas_active, 0);
}

#[tokio::test]
async fn zero_size_alloc_is_safe_and_reproducible() {
    let pool = make_pool(1, 1, 1 << 13);
    let handle = pool.acquire(128, None).await.expect("a");
    let s1: DeviceSlice<'_, u64> = handle.arena().alloc_zeroed(0).expect("zero");
    assert_eq!(s1.len(), 0);
    assert!(s1.is_empty());
    assert_eq!(s1.size_bytes(), 0);
}
