//! Phase-2 PR #12 — participant-side reload protocol.
//!
//! Triggered exclusively by SIGHUP (no participant-side gRPC reload
//! RPC in Phase 2 — OQ-PR12-1).  The participant has a narrower swap
//! surface than the controller :
//!
//!   * `iteration: Arc<ArcSwap<IterationParams>>` — swappable (α, γ
//!     knobs read by the `Step` handler).
//!   * `local: LocalParticipant`               — IMMUTABLE.  κ, dim
//!     and seed are baked into `MockParticipant` at construction time ;
//!     changing them would invalidate the Phase-1 tensor state.
//!
//! Any reload that would require mutating the participant's own row
//! is rejected with [`ServerError::Manifest`] ; the binary keeps
//! serving with the **old** `IterationParams`.  Architecture spec
//! AD3 / AD9.
//!
//! See `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 11.

use std::path::PathBuf;
use std::time::Instant;

use tracing::{error, info, instrument};

use crate::error::ServerError;
use crate::manifest::{self, find_participant, validate_manifest, Manifest};
use crate::observability::metrics::ReloadFailureReason;
use crate::participant::state::{IterationParams, ParticipantState};

/// Result of a successful participant reload.  Logged + returned to
/// the SIGHUP handler.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    /// The new monotonic generation counter (`HealthState.reload_gen`).
    pub new_reload_gen: u32,
    /// Wall-clock time of the swap, microseconds.
    pub elapsed_us: u64,
}

/// Default path for the manifest if `EFE_META_SWARM_MANIFEST` is unset.
/// Matches the Phase-2 deployment convention (Helm chart §, runbook).
const DEFAULT_MANIFEST_PATH: &str = "/etc/efe-meta-swarm/manifest.yaml";

/// Re-read manifest from disk + apply.  Called by the SIGHUP handler
/// installed in `bin/participant.rs`.
///
/// `EFE_META_SWARM_MANIFEST` is read at every reload (not cached) so
/// an operator-driven change to the env var via `kubectl set env`
/// + SIGHUP picks up the new path on the next signal.  In practice
/// the Helm chart bakes the path into the pod spec ; the dynamic
/// lookup is defence in depth.
///
/// # Errors
///
/// * [`ServerError::Config`] — env var unset (no fallback for the
///   participant ; we refuse to guess a path on the SIGHUP path) ;
/// * [`ServerError::Io`] / [`ServerError::Manifest`] — file gone or
///   invalid ;
/// * [`ServerError::Manifest`] — own row missing in the new manifest,
///   or own row drifted (AD9).
#[instrument(skip(state), fields(id = %state.local.id()))]
pub async fn apply_reload_from_disk(
    state: &ParticipantState,
) -> Result<ReloadOutcome, ServerError> {
    let path: PathBuf = std::env::var_os("EFE_META_SWARM_MANIFEST")
        .map_or_else(|| PathBuf::from(DEFAULT_MANIFEST_PATH), PathBuf::from);
    let new_manifest = match manifest::load_and_validate(&path) {
        Ok(m) => m,
        Err(e) => {
            let reason = match &e {
                ServerError::Io(_) => ReloadFailureReason::Io,
                ServerError::Manifest(_) => ReloadFailureReason::Validation,
                _ => ReloadFailureReason::Incompatible,
            };
            state.metrics.record_reload_failure(reason);
            error!(
                error = %e,
                manifest_path = %path.display(),
                reason = reason.as_str(),
                "participant reload rejected at load/validate",
            );
            return Err(e);
        }
    };
    apply_reload(state, new_manifest).await
}

/// Apply an in-memory manifest.  Inner helper ; assumes
/// `validate_manifest` has been run.
///
/// # Steps
///
/// 1. Find own row in the new manifest (must exist).
/// 2. Reject own-row drift (dim / κ / seed) — AD3 / AD9.
/// 3. Flip Health → RELOADING.
/// 4. Atomic `ArcSwap` of the iteration params.
/// 5. Flip Health → SERVING + bump `reload_gen`.
/// 6. Metrics + structured log.
///
/// `async fn` even though every step is currently sync : we keep the
/// future surface for forward-compatibility with Phase 3 weight-loader
/// reload (PR #14 will likely make this awaitable).  Callers consume
/// it as `.await` to match the controller's `apply_reload` signature
/// 1-to-1.
///
/// Span fields (architecture § 19.2) :
///   * `id` — participant id.
///   * `new_reload_gen`, `elapsed_us` — emitted in the success log.
#[allow(clippy::unused_async)]
#[instrument(skip(state, new_manifest), fields(id = %state.local.id()))]
async fn apply_reload(
    state: &ParticipantState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    let start = Instant::now();
    let own_id = state.local.id();

    // 1. Find own row.  If it's missing, the new manifest doesn't
    //    declare us — refuse the reload.
    let Some(new_row) = find_participant(&new_manifest, own_id).cloned() else {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Incompatible);
        let msg = format!(
            "reload : own participant id {own_id:?} is not present in the new manifest \
             ({} declared) ; refusing — binary keeps serving with the previous \
             manifest snapshot",
            new_manifest.participants.len(),
        );
        error!(error = %msg, "participant reload rejected");
        return Err(ServerError::Manifest(msg));
    };

    // 2. Reject own-row drift.  κ is f32 ; use the same tolerance as
    //    `validate_reload_compatibility` for parity.  Couche 3 M1.d : the
    //    manifest kappa/seed are Options (absent on efe-core rows) — the
    //    running participant stores sentinels 0.0/0 for those, so compare
    //    against `unwrap_or(sentinel)` ;  presence drift (mock ↔ efe-core)
    //    is caught by the `controller` block comparison below.
    let old_dim = state.local.dim();
    let old_kappa = state.local.kappa();
    let old_seed = state.local.seed();
    let new_dim = new_row.dim as usize;
    let new_kappa = new_row.kappa.unwrap_or(0.0);
    let new_seed = new_row.seed.unwrap_or(0);
    let kappa_drift = (old_kappa - new_kappa).abs();
    let kappa_drift_nan = kappa_drift.is_nan();
    if old_dim != new_dim
        || kappa_drift_nan
        || kappa_drift > crate::manifest::RELOAD_KAPPA_TOLERANCE
        || old_seed != new_seed
    {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Drift);
        let msg = format!(
            "reload : own row {own_id:?} drifted (dim {old_dim} → {new_dim}, \
             kappa {old_kappa} → {new_kappa} (Δ {kappa_drift}), seed {old_seed} → {new_seed}) — \
             κ / dim / seed are immutable across reload in Phase 2 (AD3 / AD9)",
        );
        error!(error = %msg, "participant reload rejected");
        return Err(ServerError::Manifest(msg));
    }

    // 2bis. Couche 3 M1.d — the efe-core `controller` block is baked in at
    //       boot (build_mvp_controller + H1 boot canary ran against IT) ;
    //       ANY drift — including presence drift (mock ↔ efe-core) — is
    //       rejected fail-closed, exactly like dim/kappa/seed.  Restart to
    //       change the engine.
    if state.local.controller_manifest() != new_row.controller.as_ref() {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Drift);
        let msg = format!(
            "reload : own row {own_id:?} controller drift — the efe-core \
             engine block is immutable across reload (the running engine and \
             its H1 certificate were built from the boot-time block) ; \
             restart the participant to change it",
        );
        error!(error = %msg, "participant reload rejected");
        return Err(ServerError::Manifest(msg));
    }

    // 3. Flip health to RELOADING — visible to ambient probes for the
    //    (~µs) swap window.
    state.health.set_reloading();

    // 4. Atomic ArcSwap of the iteration params.  In-flight Step
    //    handlers that already called `iteration.load()` keep their
    //    snapshot for the duration of the call (architecture § 12.6).
    let new_iter = IterationParams {
        alpha_default: new_manifest.iteration.alpha_default,
        coupling_gamma: new_manifest.iteration.coupling_gamma,
    };
    state.iteration_swap(new_iter);

    // 5. Flip back + bump gen + metrics.
    state.health.set_serving();
    let new_reload_gen = state.health.bump_reload_gen();
    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    state
        .metrics
        .record_reload_success(elapsed_us, new_reload_gen);

    info!(
        new_reload_gen,
        elapsed_us,
        id = own_id,
        alpha = new_manifest.iteration.alpha_default,
        gamma = new_manifest.iteration.coupling_gamma,
        "participant manifest reloaded",
    );

    Ok(ReloadOutcome {
        new_reload_gen,
        elapsed_us,
    })
}

/// Public test-only entry point.  Lets the integration tests apply a
/// pre-built manifest without an on-disk tempfile.
///
/// Production callers go through [`apply_reload_from_disk`].
///
/// # Errors
///
/// Same surface as [`apply_reload_from_disk`] minus the env-var /
/// I/O paths.  `validate_manifest` is run on `new_manifest` here so
/// callers can pass any well-formed `Manifest` without prevalidating.
#[doc(hidden)]
pub async fn apply_reload_with_manifest(
    state: &ParticipantState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    if let Err(e) = validate_manifest(&new_manifest) {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Validation);
        error!(error = %e, "participant reload rejected at validate_manifest");
        return Err(e);
    }
    apply_reload(state, new_manifest).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::participant::ParticipantConfig;
    use crate::manifest::{
        IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
    };

    fn test_config(id: &str) -> ParticipantConfig {
        ParticipantConfig {
            id: id.into(),
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            worker_threads: 1,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off by default for unit tests.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
        }
    }

    /// Build a participant state whose own row is `(id, dim=8, κ=0.6, seed=11)`.
    fn test_state(id: &str) -> ParticipantState {
        let cfg = test_config(id);
        ParticipantState::new_for_tests(cfg)
    }

    /// Build a 3-participant manifest with the same κ/dim/seed as
    /// `test_state`'s own row (so own-row drift checks pass).
    fn matching_manifest(own_id: &str) -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 16,
                safety: SafetyManifest::Box { radius: 50.0 },
            },
            participants: vec![
                ParticipantManifest {
                    id: own_id.into(),
                    dim: 8,
                    kappa: Some(0.6),
                    seed: Some(11),
                    blas_backend: "none".into(),
                    controller: None,
                },
                ParticipantManifest {
                    id: "peer-a".into(),
                    dim: 8,
                    kappa: Some(0.7),
                    seed: Some(22),
                    blas_backend: "none".into(),
                    controller: None,
                },
            ],
            iteration: IterationManifest {
                max_iters: 100,
                tolerance: 1.0e-4,
                alpha_default: 0.5,
                coupling_gamma: 0.2,
                deterministic: true,
                max_wall_ms: 15_000,
                require_convergence: false,
            },
        }
    }

    #[tokio::test]
    async fn apply_reload_swaps_alpha_and_gamma() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.iteration.alpha_default = 0.9;
        new.iteration.coupling_gamma = 0.4;

        let outcome = apply_reload_with_manifest(&state, new).await.expect("ok");
        assert_eq!(outcome.new_reload_gen, 1);

        let snap = state.iteration_snapshot();
        assert!((snap.alpha_default - 0.9).abs() < 1e-9);
        assert!((snap.coupling_gamma - 0.4).abs() < 1e-9);
        assert_eq!(state.health.snapshot().reload_gen, 1);
    }

    #[tokio::test]
    async fn apply_reload_rejects_own_kappa_drift() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.participants[0].kappa = Some(0.75); // drift from 0.6
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref m) if m.contains("drifted") && m.contains("kappa")),
            "{err:?}",
        );
        // State untouched (transactional, AD6).
        assert_eq!(state.health.snapshot().reload_gen, 0);
        let snap = state.iteration_snapshot();
        assert!((snap.alpha_default - 0.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn apply_reload_rejects_own_dim_drift() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.participants[0].dim = 16; // drift from 8
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref m) if m.contains("drifted") && m.contains("dim")),
            "{err:?}",
        );
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn apply_reload_rejects_own_seed_drift() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.participants[0].seed = Some(999); // drift from 11
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref m) if m.contains("drifted") && m.contains("seed")),
            "{err:?}",
        );
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn apply_reload_rejects_own_id_missing_in_new_manifest() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        // Remove the own row : participants[0] was "self".
        new.participants.remove(0);
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(
            matches!(err, ServerError::Manifest(ref m) if m.contains("not present in the new manifest")),
            "{err:?}",
        );
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn apply_reload_bumps_gen_on_success() {
        let state = test_state("self");
        for expected_gen in 1_u32..=3 {
            let new = matching_manifest("self");
            // Mutate alpha each iteration so the swap is observable.
            let mut n = new;
            n.iteration.alpha_default = 0.5 + 0.05 * f64::from(expected_gen);
            let outcome = apply_reload_with_manifest(&state, n).await.expect("ok");
            assert_eq!(outcome.new_reload_gen, expected_gen);
            assert_eq!(state.health.snapshot().reload_gen, expected_gen);
        }
    }

    #[tokio::test]
    async fn apply_reload_concurrent_step_sees_consistent_iteration_params() {
        let state = test_state("self");
        // Capture an "in-flight Step" snapshot.
        let pre = state.iteration_snapshot();
        assert!((pre.alpha_default - 0.5).abs() < 1e-9);

        let mut new = matching_manifest("self");
        new.iteration.alpha_default = 0.95;
        let _ = apply_reload_with_manifest(&state, new).await.expect("ok");

        // Pre-reload Arc still has the OLD values.
        assert!(
            (pre.alpha_default - 0.5).abs() < 1e-9,
            "pre-reload snapshot must keep old alpha (architecture § 12.6)",
        );
        let post = state.iteration_snapshot();
        assert!((post.alpha_default - 0.95).abs() < 1e-9);
    }

    #[tokio::test]
    async fn apply_reload_records_success_metrics() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.iteration.alpha_default = 0.65;
        let _ = apply_reload_with_manifest(&state, new).await.expect("ok");
        let body = String::from_utf8(state.metrics.render()).unwrap();
        assert!(body.contains("efe_meta_swarm_reload_attempts_total 1"));
        assert!(body.contains("efe_meta_swarm_reload_gen 1"));
    }

    #[tokio::test]
    async fn apply_reload_records_drift_failure_metrics() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.participants[0].dim = 16;
        let _ = apply_reload_with_manifest(&state, new).await.unwrap_err();
        let body = String::from_utf8(state.metrics.render()).unwrap();
        assert!(body.contains("efe_meta_swarm_reload_failures_total{reason=\"drift\"} 1"));
        assert!(body.contains("efe_meta_swarm_reload_gen 0"));
    }

    #[tokio::test]
    async fn apply_reload_rejects_nan_own_kappa() {
        let state = test_state("self");
        let mut new = matching_manifest("self");
        new.participants[0].kappa = Some(f32::NAN);
        // `validate_manifest` would catch this first, but the inner
        // `apply_reload` is also independently safe : if a future
        // refactor short-circuits the validator, the NaN guard inside
        // the own-row drift check still rejects the swap.
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
    }
}
