//! **Sprint D2 — Memory pool / arena bench (real wiring).**
//!
//! Measures the production `efe_core::memory::{MemoryPool, Arena,
//! HostBackend}` in isolation, **without any swarm/strategy mocks**,
//! to expose the per-call latency of every primitive that lives on
//! the hot path of every EFE request:
//!
//! * `MemoryPool::acquire(min_bytes, deadline)` — async, the
//!   gateway every `execute()` call goes through;
//! * `Arena::alloc_zeroed::<T>(count)` — the fundamental allocation
//!   primitive of every fusion round;
//! * `Arena::reset()` — the inter-request rollback;
//! * the realistic Mvp swarm allocation pattern (latents + masks +
//!   attention + confidence + scratch) at three fixture sizes.
//!
//! See `docs/bench/d2_memory_real_arena_architecture.md` for the full
//! design rationale (12 decisions, 9 risks, 10 SLO targets).
//!
//! # Design choices reflected in this file
//!
//! * **No `peak_alloc`** — heap profiling lives in a separate D2.5
//!   binary so it cannot pollute the latency timings here.
//! * **`BENCH_ARENA_SIZE = 1 MiB`** instead of the production 1 GiB.
//!   At 1 GiB, the implicit memset in `Arena::new` would dominate
//!   the cold-acquire bench (~330 ms per construction at 3 GB/s),
//!   making the measurement impractical. 1 MiB preserves the same
//!   *structure of cost* with sub-millisecond cold acquires. See
//!   D2 architecture §6 D2.
//! * **Multi-threaded tokio runtime, 4 workers** — matches the
//!   production server and the D1 bench harness, so the contention
//!   profile is representative.
//! * **No BLAS suffix** — this bench has no kernel that benefits
//!   from BLAS; the suffix would be misleading noise.
//!
//! # Reading the numbers
//!
//! Each group emits `<group>::<id>` keys consumed by
//! `scripts/parse_criterion_report.py` and gated by the
//! `SLO-D2-*` rules. Targets calibrated on Apple M3 Pro / 11 cores
//! / no-BLAS — CI runners get a 1.5× tolerance via the
//! `CI_COEFFICIENT` knob in `.github/workflows/bench.yml`.

#![allow(missing_docs)]
#![allow(clippy::cast_precision_loss)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::memory::{Arena, DeviceBackend, HostBackend};

use common::{make_pool, make_runtime};

// =====================================================================
// Bench-tuning constants
// =====================================================================
//
// These constants encode the deliberate divergences from the prod
// `PoolConfig::default()` motivated in the D2 architecture §6 :
//
// * `BENCH_ARENA_SIZE = 1 MiB` (vs prod 1 GiB) keeps cold-acquire
//   measurable in the 100 µs range rather than 100 ms.
// * `BENCH_DEFAULT_PREALLOC = 4` matches D1's bench tuning so the
//   warm-acquire path has 4 hits on tap.
// * `BENCH_DEFAULT_MAX = 8` matches the prod max so concurrent
//   acquire sees the same saturation cap real clients face.
// * `BENCH_MIN_BYTES_REQUEST = 1 KiB` is the smallest allocation a
//   real swarm makes (mask vector for ~256 agents). Stays below
//   `BENCH_ARENA_SIZE` for every bench so the size-filter logic of
//   `acquire` always finds a free arena.
//
// Bumping any of these without re-calibrating the SLO targets is
// a regression — `tests/pool_realism_check.rs` enforces the values
// match what the bench reports.

const BENCH_ARENA_SIZE: u64 = 1 << 20; // 1 MiB
const BENCH_DEFAULT_PREALLOC: usize = 4;
const BENCH_DEFAULT_MAX: usize = 8;
const BENCH_MIN_BYTES_REQUEST: u64 = 1 << 10; // 1 KiB

// =====================================================================
// 1. `pool_acquire_warm` — steady-state critical path
// =====================================================================
//
// Setup : 4 prealloc / 8 max (= 4 hits on tap, never saturates).
// Iter  : `pool.acquire(1 KiB, None).await` then drop.
//
// Measures the cost of the **happy path** every request pays:
// semaphore acquire (slot free) + mutex lock (free-list) + Vec
// position+swap_remove + 4 atomic counters + ArenaHandle ctor.
// Drop cost (return to pool, semaphore release) is included in the
// measurement because it's part of the per-request lifecycle.
//
// SLO-D2-1 target: p95 ≤ 1 µs (M3 Pro).

fn bench_pool_acquire_warm(c: &mut Criterion) {
    let rt = make_runtime();
    let pool = make_pool(BENCH_DEFAULT_PREALLOC, BENCH_DEFAULT_MAX, BENCH_ARENA_SIZE);

    let mut group = c.benchmark_group("pool_acquire_warm");
    group.throughput(Throughput::Elements(1));
    // 200 samples: warm acquire is sub-microsecond, low variance, so
    // many samples tighten the CI without burning wall time
    // (criterion picks ~100 k iters/sample at this scale).
    group.sample_size(200);

    group.bench_function("default", |b| {
        let pool = pool.clone();
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            async move {
                let h = pool
                    .acquire(BENCH_MIN_BYTES_REQUEST, None)
                    .await
                    .expect("warm acquire");
                // black_box keeps the handle live across the
                // optimiser's view; drop fires at scope end and
                // returns the arena to the free-list.
                black_box(&h);
            }
        });
    });
    group.finish();
}

// =====================================================================
// 2. `pool_acquire_cold` — first acquire after pool empty
// =====================================================================
//
// Setup : 0 prealloc / 1 max (every acquire that follows a release
// is technically warm, so we re-create the pool per iteration).
// Iter  : `make_pool(0,1,...)` (outside timing) → `acquire` (timed).
//
// The cost is dominated by `Arena::new`'s memset of the entire arena
// region (`HostBackend::memset_zero` over `BENCH_ARENA_SIZE` bytes).
// At 1 MiB and ~3 GB/s memset bandwidth on Apple Silicon, expect
// ~330 µs/iteration. The pool plumbing itself (semaphore, mutex,
// metrics) is < 1 % of this.
//
// We use `iter_custom` so we can run setup outside the timing window
// while keeping the timing granularity tight. `iter_batched` would
// also work but `iter_custom` is more explicit about the
// "setup→time→teardown" rhythm we want.
//
// SLO-D2-2 target: p95 ≤ 1 ms (M3 Pro).

fn bench_pool_acquire_cold(c: &mut Criterion) {
    let rt = make_runtime();
    let mut group = c.benchmark_group("pool_acquire_cold");
    group.throughput(Throughput::Elements(1));
    // Cold acquire is ~330 µs; criterion target time of 5 s default
    // gives ~15 k samples. We cap at 50 to keep wall time
    // bounded — accuracy beyond that adds little signal.
    group.sample_size(50);

    group.bench_function("1MiB", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                // Setup: fresh empty pool (no prealloc).
                let pool = make_pool(0, 1, BENCH_ARENA_SIZE);
                // Time only the acquire (which triggers Arena::new
                // because the free-list is empty).
                let start = Instant::now();
                let h = pool
                    .acquire(BENCH_MIN_BYTES_REQUEST, None)
                    .await
                    .expect("cold acquire");
                total += start.elapsed();
                black_box(&h);
                // drop(h) → arena returned to pool, then pool drops
                // outside the timed region (also outside the
                // measurement loop iteration).
            }
            total
        });
    });
    group.finish();
}

// =====================================================================
// 3. `pool_acquire_saturated` — contention behaviour
// =====================================================================
//
// Setup : 0 prealloc / 2 max (4 tasks racing for 2 slots).
// Iter  : spawn 4 tokio tasks, each acquires + releases. Wait for
//         all 4 to finish; the iteration measures the total batch.
//
// This stresses the `tokio::Semaphore` saturation path and the
// `Mutex<Vec<Arena>>` contention. Two of the four tasks must wait
// for a permit; we measure the wakeup→acquire→release pipeline on
// the contended path.
//
// SLO-D2-3 target: p95 ≤ 200 µs for the 4-task batch (~50 µs/op).

fn bench_pool_acquire_saturated(c: &mut Criterion) {
    let rt = make_runtime();
    // Pool 2/2 → first 2 tasks get the slots, last 2 must wait for
    // a release. Prealloc=0 forces fresh allocations on the first
    // two acquires; from iteration 2 onwards the free-list is warm.
    let pool = make_pool(0, 2, BENCH_ARENA_SIZE);

    let mut group = c.benchmark_group("pool_acquire_saturated");
    group.throughput(Throughput::Elements(4));
    group.sample_size(30); // contention path is high variance.

    group.bench_function("contention_4t_2slot", |b| {
        let pool = pool.clone();
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            async move {
                let mut tasks = Vec::with_capacity(4);
                for _ in 0..4 {
                    let p = pool.clone();
                    tasks.push(tokio::spawn(async move {
                        let h = p
                            .acquire(BENCH_MIN_BYTES_REQUEST, None)
                            .await
                            .expect("contention");
                        // Hold for a couple of µs — long enough for
                        // a waiter to actually enter the wait queue,
                        // short enough that the bench iteration stays
                        // sub-millisecond.
                        black_box(&h);
                    }));
                }
                for t in tasks {
                    t.await.expect("join");
                }
            }
        });
    });
    group.finish();
}

// =====================================================================
// 4. `pool_concurrent_acquire` — uncontended scaling
// =====================================================================
//
// Setup : 8 prealloc / 8 max (= 8 hits on tap, no contention on
//         the semaphore).
// Iter  : 8 concurrent tokio tasks, each acquires + drops.
//
// Difference vs `saturated` : here the semaphore is sized exactly
// to the workload, so we measure pure concurrent scalability of the
// `Mutex<Vec<Arena>>` free-list under 8-way parallel acquire/release.
//
// If a future refactor replaces the mutex with a lock-free queue
// (e.g. `crossbeam::SegQueue`), this bench should show super-linear
// improvement vs `pool_acquire_warm` on multi-core hosts.
//
// SLO-D2-3 covers this group too (same target).

const N_CONCURRENT: usize = 8;

fn bench_pool_concurrent_acquire(c: &mut Criterion) {
    let rt = make_runtime();
    let pool = make_pool(N_CONCURRENT, N_CONCURRENT, BENCH_ARENA_SIZE);

    let mut group = c.benchmark_group("pool_concurrent_acquire");
    group.throughput(Throughput::Elements(N_CONCURRENT as u64));
    group.sample_size(50);

    group.bench_function(format!("c{N_CONCURRENT}"), |b| {
        let pool = pool.clone();
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            async move {
                let mut tasks = Vec::with_capacity(N_CONCURRENT);
                for _ in 0..N_CONCURRENT {
                    let p = pool.clone();
                    tasks.push(tokio::spawn(async move {
                        let h = p
                            .acquire(BENCH_MIN_BYTES_REQUEST, None)
                            .await
                            .expect("concurrent acquire");
                        black_box(&h);
                    }));
                }
                for t in tasks {
                    t.await.expect("join");
                }
            }
        });
    });
    group.finish();
}

// =====================================================================
// 5. `arena_alloc_zeroed_typed` — typed allocation cost
// =====================================================================
//
// Setup : one arena, sized just-large-enough for the count under
//         test (×2 margin for alignment slack).
// Iter  : `arena.reset(); start = now(); arena.alloc_zeroed::<f32>(N)`.
//
// The cost decomposes into :
//
// * `cursor.fetch_add(aligned_size, AcqRel)` — ~5 ns (atomic).
// * `align_up` — ~5 ns (arithmetic).
// * `backend.memset_zero(ptr, aligned_size)` — **dominates** at
//   sizes > 1 KiB. Bandwidth-bound at ~3 GB/s on Apple Silicon,
//   ~6 GB/s on skylake-server.
// * `DeviceSlice::from_raw_parts` — ~5 ns (struct ctor).
//
// At `count=1`, the structure cost dominates (~150 ns total).
// At `count=1_000_000`, the memset dominates (~1.4 ms for 4 MB).
//
// `Throughput::Bytes` lets criterion report GB/s — directly
// readable as "what fraction of memset BW are we achieving?".
//
// SLO-D2-4/5/6 cover the 3 sizes.

fn bench_arena_alloc_zeroed_typed(c: &mut Criterion) {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());

    let mut group = c.benchmark_group("arena_alloc_zeroed_typed");

    for &count in &[1usize, 1024, 1_000_000] {
        // Arena sized for one allocation of `count × sizeof(f32)`,
        // rounded up to alignment, with ×2 margin (alignment slack
        // can push the cursor by up to 256B per alloc beyond the
        // raw size). MIN_ARENA_BYTES = 4 KiB ensures even count=1
        // has a valid arena.
        let raw_bytes = count * std::mem::size_of::<f32>();
        let arena_size = (raw_bytes
            .next_multiple_of(256)
            .max(efe_core::memory::MIN_ARENA_BYTES as usize)
            * 2) as u64;

        // Throughput in bytes lets criterion report GB/s. For count=1
        // (256 B aligned) this gives a noisy but still informative
        // BW number; for count=1M it's the headline metric.
        group.throughput(Throughput::Bytes(raw_bytes as u64));
        // Sample size adapted to per-call cost: more samples at
        // small counts (ns-scale, low cost), fewer at 1M (ms-scale).
        let samples = match count {
            1 => 100,
            1024 => 100,
            _ => 30,
        };
        group.sample_size(samples);

        group.bench_with_input(
            BenchmarkId::new("f32", count),
            &count,
            |b, &n| {
                let backend = backend.clone();
                // One arena per (count, sample-loop) — created once
                // in the outer closure, captured by move into the
                // inner FnMut closure of `iter_custom`. Each call to
                // iter_custom (criterion runs many across warmup +
                // measurement) reuses the same arena via reset.
                let mut arena =
                    Arena::new(arena_size, backend, 0).expect("bench arena");
                b.iter_custom(move |iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        // Reset is sub-50ns; not part of the timed
                        // region.
                        arena.reset();
                        let start = Instant::now();
                        let s = arena
                            .alloc_zeroed::<f32>(n)
                            .expect("alloc_zeroed");
                        total += start.elapsed();
                        black_box(s);
                    }
                    total
                });
            },
        );
    }
    group.finish();
}

// =====================================================================
// 6. `arena_reset` — cursor + generation reset cost
// =====================================================================
//
// Setup : one arena, primed with a small allocation so the cursor
//         is non-trivial.
// Iter  : `arena.reset()`.
//
// `reset` is currently :
//
// ```rust
// self.cursor.store(0, Ordering::Release);   // ~5 ns
// self.generation.fetch_add(1, Ordering::AcqRel); // ~5 ns
// ```
//
// → expected ~10 ns. SLO-D2-7 target ≤ 50 ns gives a 5× margin to
// absorb scheduler jitter and possible cache misses.
//
// **Why it matters** : if a future change makes `reset` zero the
// arena (security feature against cross-tenant residue), this bench
// will scream — and the impact ripples through every fusion round.

fn bench_arena_reset(c: &mut Criterion) {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());

    let mut group = c.benchmark_group("arena_reset");
    group.throughput(Throughput::Elements(1));
    // ns-scale operation — many samples, many iters/sample.
    group.sample_size(200);

    group.bench_function("default", move |b| {
        let mut arena =
            Arena::new(BENCH_ARENA_SIZE, backend.clone(), 0).expect("arena");
        // Prime cursor so reset has something to reset to 0 (otherwise
        // the bench runs against an arena that has never seen an
        // allocation, which is unrepresentative).
        let _ = arena.alloc_zeroed::<u8>(4096).expect("warmup alloc");

        b.iter(|| {
            arena.reset();
            black_box(&arena);
        });
    });
    group.finish();
}

// =====================================================================
// 7. `realistic_swarm_alloc_pattern` — Mvp fusion round emulation
// =====================================================================
//
// Reproduces the exact allocation sequence one round of `MvpCpuFusion`
// performs, in the right order, with the right sizes :
//
// 1. `latents : f32[n_agents × dim]`         — epistemic state
// 2. `mask : u8[n_agents]`                    — active flag
// 3. `attention : f32[n_agents²]`             — pairwise similarities
// 4. `confidence : f32[n_agents]`             — message weights
// 5. `scratch : f32[n_agents × dim]`          — softmax / drift buf
//
// Three fixtures matched on D1 v2 specs:
//
// | name   | dim | n_agents | total bytes (raw, pre-alignment) |
// |--------|----:|---------:|---------------------------------:|
// | small  | 128 | 8        | ~12 KiB                          |
// | medium | 256 | 32       | ~70 KiB                          |
// | large  | 256 | 128      | ~330 KiB                         |
//
// All three fit in `BENCH_ARENA_SIZE = 1 MiB` with ×3 margin.
//
// The bench measures **acquire + 5 alloc_zeroed + drop** per
// iteration — the realistic memory cost of one fusion round, as
// seen from outside the round itself. Combined with the per-phase
// numbers from groups 1-5 above, this bench validates the additivity
// hypothesis : realistic ≈ acquire_warm + Σ alloc_zeroed.
//
// SLO-D2-8/9/10 cover the three fixtures.

fn bench_realistic_swarm_alloc_pattern(c: &mut Criterion) {
    let rt = make_runtime();
    let pool = make_pool(BENCH_DEFAULT_PREALLOC, BENCH_DEFAULT_MAX, BENCH_ARENA_SIZE);

    let mut group = c.benchmark_group("realistic_swarm_alloc_pattern");
    group.throughput(Throughput::Elements(1));

    // (name, dim, n_agents) — matched on the D1 v2 fixture sizes.
    let fixtures: &[(&str, usize, usize)] = &[
        ("small", 128, 8),
        ("medium", 256, 32),
        ("large", 256, 128),
    ];

    for &(name, dim, n_agents) in fixtures {
        // Adjust sample_size by fixture size: small is sub-10µs,
        // large is ~100µs — proportionally fewer samples on large
        // keeps wall time roughly constant across fixtures.
        let samples = match name {
            "small" => 100,
            "medium" => 50,
            _ => 50,
        };
        group.sample_size(samples);

        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(dim, n_agents),
            |b, &(dim, n_agents)| {
                let pool = pool.clone();
                b.to_async(&rt).iter(|| {
                    let pool = pool.clone();
                    async move {
                        // The 1024 min_bytes is dwarfed by the
                        // arena size; the size-filter always matches
                        // the first free arena.
                        let mut handle = pool
                            .acquire(BENCH_MIN_BYTES_REQUEST, None)
                            .await
                            .expect("realistic acquire");

                        // **Critical**: reset the arena before the
                        // allocation pass. The pool's
                        // `return_arena` does NOT reset the cursor
                        // (cf. pool.rs::MemoryPoolInner::return_arena);
                        // the cursor of a re-acquired arena reflects
                        // wherever the previous request left it. In
                        // production this reset is performed by the
                        // genesis layer (`DefaultSpawner` resets
                        // before its first allocation). The bench
                        // mirrors that exactly so each iteration
                        // starts from cursor=0 — otherwise we'd OOM
                        // after roughly `BENCH_ARENA_SIZE /
                        // request_bytes` iterations.
                        //
                        // The reset itself is sub-50 ns (cf.
                        // `bench_arena_reset`), so it does not
                        // materially distort the realistic-pattern
                        // measurement. We deliberately include it
                        // *inside* the timed region: every real
                        // request pays for a reset, so the bench
                        // should too. The borrow-checker dance
                        // (`arena_mut()` → drop borrow → `arena()`)
                        // is handled by NLL — the `&mut` only lives
                        // until the semicolon.
                        handle.arena_mut().reset();
                        let arena = handle.arena();

                        // Order matters — must mirror MvpCpuFusion
                        // (cf. D2 architecture §5.7 and §7 R5).
                        // Each `alloc_zeroed` borrows from `arena`;
                        // all 5 slices coexist until end-of-block.
                        let latents = arena
                            .alloc_zeroed::<f32>(n_agents * dim)
                            .expect("latents");
                        let mask = arena
                            .alloc_zeroed::<u8>(n_agents)
                            .expect("mask");
                        let attention = arena
                            .alloc_zeroed::<f32>(n_agents * n_agents)
                            .expect("attention");
                        let confidence = arena
                            .alloc_zeroed::<f32>(n_agents)
                            .expect("confidence");
                        let scratch = arena
                            .alloc_zeroed::<f32>(n_agents * dim)
                            .expect("scratch");

                        black_box((
                            &latents,
                            &mask,
                            &attention,
                            &confidence,
                            &scratch,
                        ));
                        // Slices drop here (no-op, they just borrow).
                        // Then handle drops, returning the arena to
                        // the pool with the cursor at the high-water
                        // mark. The next iteration's `reset()` sets
                        // it back to 0, hence the loop is sound.
                    }
                });
            },
        );
    }
    group.finish();
}

// =====================================================================
// Criterion entry points
// =====================================================================
//
// Note on group ordering : criterion runs groups in the order they
// appear in `targets`. We list "cheap, low-variance" groups first
// (warm acquire, reset) so an early failure is detected fast, before
// the expensive cold/realistic groups run.

criterion_group!(
    name = memory_real_arena_benches;
    config = Criterion::default()
        // Memory benches share criterion's default 5 % significance
        // level; concurrency-heavy groups bump noise_threshold to
        // 12 % via group-level config (see saturated/concurrent).
        .significance_level(0.05);
    targets =
        bench_pool_acquire_warm,
        bench_arena_reset,
        bench_arena_alloc_zeroed_typed,
        bench_pool_concurrent_acquire,
        bench_pool_acquire_saturated,
        bench_pool_acquire_cold,
        bench_realistic_swarm_alloc_pattern,
);
criterion_main!(memory_real_arena_benches);
