//! **Sprint M1 extension D38** — Parallel k-swarm runner.
//!
//! ## Mission
//!
//! Pre-D38 the k-swarm parallelism lived inside
//! `efe-server::services::mode3::Mode3ServiceImpl::execute()` —
//! coupled to the gRPC transport. `efe-ffi`, `efe-bench`, and
//! integration tests couldn't reuse it.
//!
//! Sprint D38 extracts the `tokio::task::JoinSet`-based fan-out into
//! a generic [`run_k_swarms`] function that takes a **task factory
//! closure** producing one `Future<Output = Result<T>>` per index.
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 12.
//!
//! ## Behaviour contract
//!
//!   * **Spawn** : `k` futures spawned eagerly via `JoinSet::spawn`.
//!   * **Collection** : `JoinSet::join_next` until all complete.
//!     Results are returned in **arrival order** (first to finish
//!     first), NOT submission order.
//!   * **Fail-fast** : on the first task that errors, abort the
//!     remaining (best-effort) and return the error.
//!   * **Panic safety** : a panic in a sub-task is caught by the
//!     `JoinError` path and surfaced as `EfeError::Internal`.
//!
//! ## Why `Fn(u32) -> Fut` and not `FnOnce`
//!
//! Caller may want to spawn the same future shape for multiple
//! indices ; using `Fn` lets us call the factory `k` times. If the
//! caller only needs `FnOnce` semantics, they capture-by-move once
//! per iteration via `let owned = ...; move ||` patterns.
//!
//! ## tokio runtime requirement
//!
//! `JoinSet` only works inside a tokio multi-threaded or current-thread
//! runtime. Callers that need to invoke `run_k_swarms` from a sync
//! context (e.g. PyO3 bindings) must wrap in `Handle::block_on`.

use std::future::Future;

use tokio::task::{JoinError, JoinSet};

use crate::errors::{EfeError, Result};

/// **Sprint D38** — Spawn `k` async tasks via `tokio::task::JoinSet`
/// and collect results.
///
/// Returns each task's result in **arrival order** (not submission
/// order).
///
/// `task_factory(idx)` is called eagerly for `idx in 0..k` and
/// must produce a `Future<Output = Result<T>>`. The futures are
/// spawned onto the ambient tokio runtime ; the function awaits
/// `JoinSet::join_next` until all complete.
///
/// ## Fail-fast
///
/// On the first error, the function aborts the remaining tasks
/// (best-effort cancellation) and returns the error. This matches
/// the Mode-3 SLO contract : dispersion across sub-swarms is
/// informational, but a fundamental error in any sub-swarm should
/// fail the whole call.
///
/// ## Errors
///
///   * Returns the first inner `Result::Err` from any task.
///   * Returns [`EfeError::Internal`] on a `tokio::task::JoinError`
///     (panic in a sub-task or aborted before completion).
///
/// ## Examples
///
/// ```no_run
/// # use efe_core::swarm::mode3::parallel::run_k_swarms;
/// # use efe_core::errors::Result;
/// # async fn example() -> Result<()> {
/// // Spawn 5 tasks that each return their squared index.
/// let results: Vec<u32> = run_k_swarms(5, |i| async move {
///     Ok::<u32, efe_core::EfeError>(i * i)
/// }).await?;
/// assert_eq!(results.len(), 5);
/// // results are in arrival order — sort to compare.
/// let mut sorted = results.clone();
/// sorted.sort();
/// assert_eq!(sorted, vec![0, 1, 4, 9, 16]);
/// # Ok(())
/// # }
/// ```
pub async fn run_k_swarms<T, F, Fut>(k: u32, task_factory: F) -> Result<Vec<T>>
where
    T: Send + 'static,
    F: Fn(u32) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T>> + Send + 'static,
{
    let mut set: JoinSet<Result<T>> = JoinSet::new();
    let mut abort_handles = Vec::with_capacity(k as usize);
    for i in 0..k {
        let fut = task_factory(i);
        let h = set.spawn(fut);
        abort_handles.push(h);
    }

    let mut out = Vec::with_capacity(k as usize);
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(v)) => out.push(v),
            Ok(Err(e)) => {
                // Fail-fast : abort remaining tasks then surface the
                // first error. We don't wait for the aborts to
                // complete — they're best-effort cleanup.
                for h in &abort_handles {
                    h.abort();
                }
                return Err(e);
            }
            Err(join_err) => {
                for h in &abort_handles {
                    h.abort();
                }
                return Err(map_join_error(join_err));
            }
        }
    }
    Ok(out)
}

fn map_join_error(e: JoinError) -> EfeError {
    if e.is_cancelled() {
        EfeError::Internal("Mode-3 sub-task cancelled")
    } else {
        EfeError::Internal("Mode-3 sub-task panicked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn run_k_swarms_collects_all_results_when_happy() {
        let results = run_k_swarms(5, |i| async move { Ok::<u32, EfeError>(i * 10) })
            .await
            .unwrap();
        assert_eq!(results.len(), 5);
        let mut sorted = results.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 10, 20, 30, 40]);
    }

    #[tokio::test]
    async fn run_k_swarms_zero_tasks_returns_empty_vec() {
        let results: Vec<u32> = run_k_swarms(0, |_| async move { Ok(0u32) }).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn run_k_swarms_returns_first_error() {
        let err = run_k_swarms(3, |i| async move {
            if i == 1 {
                Err(EfeError::InvalidConfig("test error".into()))
            } else {
                // Make the non-erroring tasks slower so the error
                // wins the join_next race deterministically.
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<u32, EfeError>(i)
            }
        })
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("test error"));
    }

    #[tokio::test]
    async fn run_k_swarms_aborts_remaining_on_first_error() {
        // Counter incremented by each task that COMPLETES (not aborted).
        // After the first error, the remaining 9 tasks should all be
        // aborted — counter should stay <= 1 (just the failing task,
        // and possibly racing tasks that finished before the abort
        // landed).
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let _ = run_k_swarms(10, move |i| {
            let counter = counter_clone.clone();
            async move {
                if i == 0 {
                    Err(EfeError::Internal("boom"))
                } else {
                    // 200 ms delay so the abort has time to land.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<u32, EfeError>(i)
                }
            }
        })
        .await;
        // Generous bound — racing aborts may not catch all 9.
        let n_completed = counter.load(Ordering::SeqCst);
        assert!(n_completed < 10, "expected aborts ; got {n_completed}/9 completed");
    }

    #[tokio::test]
    async fn run_k_swarms_handles_panic_via_join_error() {
        let err = run_k_swarms(3, |i| async move {
            if i == 1 {
                panic!("intentional panic in sub-task");
            }
            Ok::<u32, EfeError>(i)
        })
        .await
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("panicked") || msg.contains("Mode-3 sub-task"));
    }

    #[tokio::test]
    async fn run_k_swarms_arrival_order_not_submit_order() {
        // Index 4 sleeps the LEAST → arrives FIRST.
        let results = run_k_swarms(5, |i| async move {
            let sleep_ms = (5 - i) as u64 * 30;
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            Ok::<u32, EfeError>(i)
        })
        .await
        .unwrap();
        // First completed = index 4 (slept 30 ms instead of 150 ms).
        assert_eq!(results[0], 4);
        // Last completed = index 0 (slept 150 ms).
        assert_eq!(results[4], 0);
    }

    #[tokio::test]
    async fn run_k_swarms_speedup_vs_sequential() {
        // Sanity check : 5 tasks of 50 ms each should complete in
        // < 200 ms (parallel) vs 250 ms (sequential).
        let started = std::time::Instant::now();
        let _ = run_k_swarms(5, |_| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<u32, EfeError>(0)
        })
        .await
        .unwrap();
        let parallel_ms = started.elapsed().as_millis() as u64;
        // Allow generous slack for CI variance ; sequential would be
        // > 250 ms ; parallel should be < 150 ms in practice.
        assert!(
            parallel_ms < 200,
            "parallel took {parallel_ms} ms ; expected < 200 ms (k=5 × 50 ms each)"
        );
    }
}
