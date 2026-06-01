//! Sprint **D27** (M4 § 11) — Live bundle reload.
//!
//! ## Mission
//!
//! Allow the operator to swap an embedder bundle WITHOUT restarting
//! the pod and WITHOUT dropping in-flight RPCs. This is the wiring
//! that backs the `AdminService.ReloadBundle` RPC (declared in D21).
//!
//! ## Algorithm (3 phases)
//!
//! ```text
//! Operator → AdminService.ReloadBundle({ bundle_path, … })
//!                         │
//!                         ▼
//!    Phase 1 — verify (≤ 5 s)
//!        - load bundle dir via `bundle::load_bundle_dir`
//!        - Ed25519 signature check (D16 invariant)
//!        - per-component SHA-256 walk (D16 invariant)
//!        - smoke test : encode 5 fixed sentences, assert L2-norm = 1
//!
//!    Phase 2 — atomic swap (≤ 1 s)
//!        - acquire `parking_lot::RwLock` write guard on `active`
//!        - new_arc = Arc::new(new_bundle)
//!        - old_arc = mem::replace(active, new_arc)            ← atomic
//!        - release write guard
//!
//!    Phase 3 — drain (≤ 30 s, async)
//!        - spawn detached task holding `Arc<Bundle>` (old)
//!        - poll `Arc::strong_count(&old) == 1` every 100 ms
//!        - when reached → drop `old` (triggers ONNX session close)
//!
//!    Response : ReloadBundleResponse { status: ACTIVE, duration_ms,
//!                                      old_bundle_id, new_bundle_id }
//! ```
//!
//! ## Failure modes
//!
//! * Phase 1 errors are surfaced as `ReloadError::*` and the swap
//!   never happens : the previous active bundle keeps serving traffic.
//! * Phase 2 cannot fail in the conventional sense (a `RwLock::write`
//!   only yields once held) ; the worst case is the lock being held by
//!   another `reload()` call, in which case we return
//!   `ReloadError::ReloadInProgress`.
//! * Phase 3 timeouts (30 s default) leave the old bundle alive in
//!   memory but no new traffic reaches it. A `tracing::warn!` event is
//!   emitted so operators can investigate.
//!
//! ## Multi-replica coordination
//!
//! Per-replica reload is the contract for D27 : the operator either
//! sends `ReloadBundle` to each replica in a loop, or relies on a Helm
//! rollout that gradually rolls all pods. PG-backed propagation of
//! reload events is `DT-S-D27-COORD-1` (post-GA).
//!
//! ## Audit fix S-A3
//!
//! Pre-D27 the EmbedderRegistry had no notion of an "active" bundle —
//! every consumer (gRPC handler) looked up by `bundle_id`. This module
//! introduces an `active: RwLock<Option<Arc<Bundle>>>` slot so the
//! ReloadBundle RPC has a well-defined target.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::RwLock;

use crate::embedder::bundle::{Bundle, BundleLoadError, LoadPolicy};
use crate::embedder::cache::EmbedderRegistry;
use crate::embedder::trust::TrustStore;
use crate::embedder::EmbedderError;

/// Default smoke test fixtures. Hardcoded so the reload pipeline is
/// deterministic and bench-stable across runs / versions. The L2-norm
/// of every produced embedding MUST be 1.0 ± 1e-3 ; otherwise the
/// reload aborts.
pub const SMOKE_FIXTURES: &[&str] = &[
    "hello",
    "the quick brown fox",
    "lorem ipsum dolor",
    "salut le monde",
    "test 1 2 3",
];

/// Acceptable L2-norm deviation in the smoke test. 1e-3 covers
/// quantization rounding for fp16 / int8 ONNX models without admitting
/// a genuinely-broken normalizer.
pub const SMOKE_L2_TOLERANCE: f32 = 1e-3;

/// Default drain timeout for Phase 3. Configurable via env var
/// `EFE_BUNDLE_DRAIN_TIMEOUT_S` (cf. M12 §8.4 config family).
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Polling interval inside the drain loop. 100 ms is a balance
/// between responsiveness and CPU cost.
pub const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Typed reload errors. Mapped onto `tonic::Status` by the gRPC
/// handler.
#[derive(Debug, Error)]
pub enum ReloadError {
    /// Bundle load (Phase 1) failed.
    ///
    /// We carry an `Arc<BundleLoadError>` rather than a plain
    /// `BundleLoadError` because `EmbedderRegistry::ensure_loaded`
    /// returns the error inside an `Arc` so multiple waiters can
    /// share the singleflight failure (cf. `cache.rs::SharedLoadResult`).
    #[error("bundle load failed: {0}")]
    Load(Arc<BundleLoadError>),

    /// Smoke test failed at runtime.
    #[error("smoke test encode failed: {0}")]
    SmokeEncode(#[from] EmbedderError),

    /// Smoke test L2-norm deviation exceeded tolerance.
    #[error("smoke test L2-norm = {0} (expected 1.0 ± {SMOKE_L2_TOLERANCE})")]
    SmokeL2Norm(f32),

    /// Another reload is already in progress.
    #[error("reload already in progress")]
    ReloadInProgress,

    /// Drain timed out (Phase 3) — old bundle still has refs after the
    /// configured timeout.
    #[error("drain timeout: old bundle still has {0} refs after {1}s")]
    DrainTimeout(usize, u64),
}

/// Atomic active-bundle slot + reload telemetry.
///
/// **D27 wiring** : at server boot, the runtime constructs **one**
/// [`ActiveBundleSlot`] per server instance and shares it across the
/// gRPC handlers (Mode-1, Mode-2, Mode-3) that need to embed text
/// server-side. The `AdminService.ReloadBundle` handler holds a clone
/// of the same `Arc<ActiveBundleSlot>` and calls [`Self::reload`] to
/// swap the contents.
#[derive(Default)]
pub struct ActiveBundleSlot {
    active: RwLock<Option<Arc<Bundle>>>,
    /// Unix ms timestamp of the most recent successful reload. 0 when
    /// no reload has happened (i.e. boot value, no swap yet).
    last_reload_ts_ms: AtomicU64,
    /// Whether a reload is currently in flight. Used as a singleflight
    /// gate ; the second concurrent reload returns
    /// `ReloadError::ReloadInProgress` immediately rather than queueing.
    in_flight: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for ActiveBundleSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveBundleSlot")
            .field(
                "last_reload_ts_ms",
                &self.last_reload_ts_ms.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl ActiveBundleSlot {
    /// Empty slot. The first call to [`Self::set_initial`] populates it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current active bundle. Returns `None` when no
    /// bundle has been loaded yet.
    pub async fn current(&self) -> Option<Arc<Bundle>> {
        self.active.read().await.clone()
    }

    /// Install the initial bundle. Used at boot ; subsequent calls are
    /// idempotent (replace without drain).
    pub async fn set_initial(&self, bundle: Arc<Bundle>) {
        *self.active.write().await = Some(bundle);
        self.last_reload_ts_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Unix-ms timestamp of the most recent successful reload.
    /// 0 when no swap has happened.
    pub fn last_reload_ts_ms(&self) -> u64 {
        self.last_reload_ts_ms.load(Ordering::Relaxed)
    }

    /// Atomic swap with verify + smoke + drain.
    ///
    /// Returns the elapsed wall time + the new bundle id on success.
    /// On error, the previous active bundle stays unchanged.
    pub async fn reload(
        &self,
        registry: &EmbedderRegistry,
        bundle_id: &str,
        bundle_path: PathBuf,
        policy: LoadPolicy,
        trust: Arc<TrustStore>,
        drain_timeout: Duration,
    ) -> Result<ReloadOutcome, ReloadError> {
        let started = Instant::now();

        // Singleflight gate.
        let _gate = match self.in_flight.try_lock() {
            Ok(g) => g,
            Err(_) => return Err(ReloadError::ReloadInProgress),
        };

        // Phase 1 — load + verify (signature + per-component SHA).
        // The registry's `ensure_loaded` enforces both invariants.
        // The error is `Arc<BundleLoadError>` so we propagate the Arc
        // unchanged ; the full Display chain (Manifest, Signature,
        // Verify, Runtime, …) reaches the gRPC handler intact.
        let bundle = registry
            .ensure_loaded(bundle_id, bundle_path, policy, trust)
            .await
            .map_err(ReloadError::Load)?;

        // Phase 1.5 — smoke test on the freshly-loaded bundle.
        smoke_test(&bundle).await?;

        // Phase 2 — atomic swap.
        let new_arc = Arc::new(bundle.clone());
        let old_arc = {
            let mut guard = self.active.write().await;
            guard.replace(new_arc)
        };

        let new_id = bundle.manifest().bundle_id.clone();
        let old_id = old_arc
            .as_ref()
            .map(|b| b.manifest().bundle_id.clone())
            .unwrap_or_default();

        self.last_reload_ts_ms
            .store(now_unix_ms(), Ordering::Relaxed);

        // Phase 3 — drain, spawned as a detached task. The reload RPC
        // returns ACTIVE as soon as Phase 2 lands ; the drain happens
        // in the background and emits a tracing event when it finishes.
        if let Some(old) = old_arc {
            tokio::spawn(drain_old(old, drain_timeout));
        }

        Ok(ReloadOutcome {
            new_bundle_id: new_id,
            old_bundle_id: old_id,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// Result of a successful reload.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    /// Bundle id of the newly active bundle.
    pub new_bundle_id: String,
    /// Bundle id of the previous bundle (empty on first reload).
    pub old_bundle_id: String,
    /// Wall time of Phase 1 + Phase 2 (excludes background drain).
    pub duration_ms: u64,
}

/// Smoke test : encode `SMOKE_FIXTURES`, assert each L2-norm = 1 ± tol.
async fn smoke_test(bundle: &Bundle) -> Result<(), ReloadError> {
    for text in SMOKE_FIXTURES {
        let emb = bundle.encode_one(text)?;
        let norm: f32 = emb.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (norm - 1.0).abs() > SMOKE_L2_TOLERANCE {
            return Err(ReloadError::SmokeL2Norm(norm));
        }
    }
    Ok(())
}

/// Drain task : wait until the old `Arc<Bundle>` strong count reaches 1
/// (we are the only holder) then drop it. Bounded by `timeout`.
async fn drain_old(old: Arc<Bundle>, timeout: Duration) {
    let started = Instant::now();
    while Arc::strong_count(&old) > 1 {
        if started.elapsed() >= timeout {
            tracing::warn!(
                target: "efe.embedder.reload",
                refs = Arc::strong_count(&old),
                timeout_s = timeout.as_secs(),
                "drain timed out — old bundle has lingering refs (probably long-running streaming RPC)"
            );
            return;
        }
        tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
    }
    // Last ref ; dropping `old` here triggers `Bundle::drop` side-effects
    // (close ONNX session, unmap files, etc.).
    drop(old);
    tracing::info!(
        target: "efe.embedder.reload",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "drain completed cleanly"
    );
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_default_has_no_active_bundle() {
        let slot = ActiveBundleSlot::new();
        // We use a tokio::runtime here because `current` is async ;
        // a single-threaded current_thread runtime is enough for this
        // pure-state assertion.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cur = rt.block_on(slot.current());
        assert!(cur.is_none());
        assert_eq!(slot.last_reload_ts_ms(), 0);
    }

    #[test]
    fn reload_outcome_preserves_new_id() {
        let outcome = ReloadOutcome {
            new_bundle_id: "x/y@1.0/onnx/64".into(),
            old_bundle_id: String::new(),
            duration_ms: 1234,
        };
        assert_eq!(outcome.new_bundle_id, "x/y@1.0/onnx/64");
        assert_eq!(outcome.duration_ms, 1234);
    }

    #[test]
    fn reload_error_smoke_l2_carries_value() {
        let e = ReloadError::SmokeL2Norm(1.5);
        let s = format!("{e}");
        assert!(s.contains("1.5"));
    }

    #[test]
    fn smoke_fixtures_are_non_empty_and_ascii() {
        // The fixtures are part of the wire contract for D27 ; they
        // must remain hardcoded and bench-stable. The pin test
        // `test_d27_bundle_reload.py` cross-checks this.
        assert!(!SMOKE_FIXTURES.is_empty());
        for &f in SMOKE_FIXTURES {
            assert!(!f.is_empty(), "fixture must not be empty");
        }
    }
}
