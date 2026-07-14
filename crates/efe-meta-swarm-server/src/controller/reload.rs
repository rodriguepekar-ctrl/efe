//! Phase-2 PR #12 — controller-side reload protocol.
//!
//! Two trigger paths share this module's `apply_reload` :
//!   * **SIGHUP** : `bin/controller.rs` installs the handler, which
//!     calls [`apply_reload_from_disk`] on every signal.
//!   * **gRPC** : `ControllerService::reload_config` (PR #12 C4 ;
//!     defined in the sibling private `service` module) calls
//!     [`apply_reload_with_manifest`] directly with the inline
//!     payload, or [`apply_reload_from_disk`] when the payload is
//!     empty (the operator edited the file and just wants to reload).
//!
//! See `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 7-9 for the design and
//! § 19 for the metrics surface.

use std::time::Instant;

use tracing::{error, info, instrument};

use crate::controller::state::ControllerState;
use crate::error::ServerError;
use crate::manifest::{
    self, validate_manifest, validate_reload_compatibility, Manifest, ParticipantManifest,
    SafetyManifest,
};
use crate::observability::metrics::ReloadFailureReason;

/// Result of a successful reload attempt.  Returned by both
/// `apply_reload_*` variants and surfaced in `ReloadConfigResponse` /
/// SIGHUP log lines.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    /// The new monotonic generation counter (`HealthState.reload_gen`).
    pub new_reload_gen: u32,
    /// Number of participants in the new manifest.
    pub participants: usize,
    /// Wall-clock time of the swap, microseconds.
    pub elapsed_us: u64,
    /// Human-readable diff between old and new manifest.  Travels in
    /// `ReloadConfigResponse.applied_changes`.  Sample entries :
    ///   * `"iteration.alpha_default: 0.5 → 0.7"`
    ///   * `"participants: added fraud-detection"`
    pub applied_changes: Vec<String>,
}

/// Re-read the manifest from `state.config.manifest_path` and apply it.
/// Used by SIGHUP — the operator's source of truth is the on-disk file.
///
/// # Errors
///
/// Returns [`ServerError::Manifest`] / [`ServerError::Io`] if the file
/// can't be loaded, can't be validated, or is incompatible with the
/// running state.  In every error case, the running state is
/// **untouched** (transactional ; cf. AD6).
#[instrument(skip(state))]
pub async fn apply_reload_from_disk(state: &ControllerState) -> Result<ReloadOutcome, ServerError> {
    let path = state.config.manifest_path.clone();
    // PR #18 Span 7 — wrap the whole disk-reload flow in a
    // manifest.reload span (trigger=sighup) so failures on the
    // load/validate path are also captured.  G9 gap-fill.
    let span_start = Instant::now();
    let gen_before = state.health.snapshot().reload_gen;
    let participants_before = state.manifest.read().await.participants.len();
    let reload_span = crate::tracing_otel::spans::span_manifest_reload(
        "sighup",
        gen_before,
        participants_before,
    );

    let new_manifest = match manifest::load_and_validate(&path) {
        Ok(m) => m,
        Err(e) => {
            // Classify the failure for the failures_total label.
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
                "reload rejected at load/validate",
            );
            // PR #18 — record failure-path close attributes on the
            // manifest.reload span.
            crate::tracing_otel::spans::record_manifest_reload_close(
                &reload_span,
                gen_before,
                participants_before,
                "error",
                Some(reason.as_str()),
                0,
                span_start.elapsed().as_secs_f64(),
            );
            crate::tracing_otel::spans::record_grpc_error(&reload_span, &format!("{e}"));
            return Err(e);
        }
    };
    // Delegate to the success-path span recorder ; the existing
    // `apply_reload` records its own span with the post-swap
    // attributes.  Drop our outer span here so we don't double-emit.
    // Audit-pass : thread the SIGHUP trigger through `apply_reload`
    // so the success-path span isn't mislabeled "grpc".
    drop(reload_span);
    apply_reload(state, new_manifest, "sighup").await
}

/// Apply an in-memory new manifest.  Used by the gRPC RPC :
///   * empty request body → caller calls [`apply_reload_from_disk`]
///     instead (no manifest synthesis on the caller side) ;
///   * non-empty request body → caller builds the new manifest from
///     the request payload via [`build_inline_manifest`], then calls
///     this.
///
/// The supplied `new_manifest` is **re-validated** by this function
/// against `validate_manifest` before any state mutation, so callers
/// don't need to revalidate on their own.
#[instrument(skip(state, new_manifest), fields(participants = new_manifest.participants.len()))]
pub async fn apply_reload_with_manifest(
    state: &ControllerState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    // PR #18 Span 7 — wrap the gRPC-driven reload in a span
    // (trigger=grpc).  G9 gap-fill.
    let span_start = Instant::now();
    let gen_before = state.health.snapshot().reload_gen;
    let participants_before = state.manifest.read().await.participants.len();
    let reload_span = crate::tracing_otel::spans::span_manifest_reload(
        "grpc",
        gen_before,
        participants_before,
    );

    if let Err(e) = validate_manifest(&new_manifest) {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Validation);
        error!(error = %e, "reload rejected at validate_manifest");
        crate::tracing_otel::spans::record_manifest_reload_close(
            &reload_span,
            gen_before,
            participants_before,
            "error",
            Some(ReloadFailureReason::Validation.as_str()),
            0,
            span_start.elapsed().as_secs_f64(),
        );
        crate::tracing_otel::spans::record_grpc_error(&reload_span, &format!("{e}"));
        return Err(e);
    }
    drop(reload_span);
    apply_reload(state, new_manifest, "grpc").await
}

/// Shared inner path.  Assumes `new_manifest` already passed
/// `validate_manifest`.  Now :
///   1. checks reload-compatibility against the current manifest ;
///   2. flips `HealthState` to RELOADING ;
///   3. acquires the write lock + swaps ;
///   4. flips `HealthState` back to SERVING + bumps `reload_gen`.
///
/// Bumping happens **only on success** so a failed reload doesn't
/// move the operator's monotonic confirmation signal (AD7).
///
/// Span fields (architecture § 19.2) :
///   * `participants` — incoming manifest's participant count.
///   * `new_reload_gen` — post-bump gen (recorded in the success log).
///   * `elapsed_us` — write-lock window duration (recorded in the
///     success log).
#[instrument(
    skip(state, new_manifest),
    fields(participants = new_manifest.participants.len(), trigger = %trigger),
)]
async fn apply_reload(
    state: &ControllerState,
    new_manifest: Manifest,
    trigger: &'static str,
) -> Result<ReloadOutcome, ServerError> {
    let start = Instant::now();

    // 1. Reload-specific compatibility check.  Done OUTSIDE the write
    //    lock so a rejected reload doesn't block readers (architecture
    //    § 9.2).  Validation runs against a *clone* of the current
    //    Manifest so the read guard's lifetime is the minimum possible.
    let (compat_check, diff) = {
        let current = state.manifest.read().await;
        let compat = validate_reload_compatibility(&current, &new_manifest);
        let diff = diff_manifests(&current, &new_manifest);
        (compat, diff)
    }; // read guard dropped here.
    if let Err(e) = compat_check {
        state
            .metrics
            .record_reload_failure(ReloadFailureReason::Drift);
        error!(error = %e, "reload rejected at validate_reload_compatibility");
        // PR #18 — emit a manifest.reload span on the drift-failure
        // path.  G9 gap-fill.  Audit-pass : `trigger` is now threaded
        // from the outer wrapper so SIGHUP-triggered drift failures
        // get the correct trigger label.
        let gen_before = state.health.snapshot().reload_gen;
        let participants_before = {
            let m = state.manifest.read().await;
            m.participants.len()
        };
        let span = crate::tracing_otel::spans::span_manifest_reload(
            trigger,
            gen_before,
            participants_before,
        );
        crate::tracing_otel::spans::record_manifest_reload_close(
            &span,
            gen_before,
            participants_before,
            "error",
            Some(ReloadFailureReason::Drift.as_str()),
            0,
            start.elapsed().as_secs_f64(),
        );
        crate::tracing_otel::spans::record_grpc_error(&span, &format!("{e}"));
        return Err(e);
    }

    // 2. Flip health to RELOADING — visible to ambient probes for the
    //    window between this store and the SERVING flip below.
    state.health.set_reloading();

    // 3. Acquire write lock + atomic swap.
    let participants_count = new_manifest.participants.len();
    {
        let mut write_guard = state.manifest.write().await;
        *write_guard = new_manifest;
    } // write guard dropped — readers can proceed with the new manifest.

    // 4. Flip back + bump gen + metrics.
    state.health.set_serving();
    let new_reload_gen = state.health.bump_reload_gen();
    let elapsed = start.elapsed();
    let elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    // PR #12 metrics (preserved).
    state
        .metrics
        .record_reload_success(elapsed_us, new_reload_gen);
    // PR #18 Span 7 — manifest.reload span (success path).  Audit-pass :
    // `trigger` now reflects the actual outer trigger ("sighup" /
    // "grpc") rather than the previous always-"grpc" stub.
    let reload_span = crate::tracing_otel::spans::span_manifest_reload(
        trigger,
        new_reload_gen.saturating_sub(1),
        participants_count,
    );
    crate::tracing_otel::spans::record_manifest_reload_close(
        &reload_span,
        new_reload_gen,
        participants_count,
        "ok",
        None,
        diff.len(),
        elapsed.as_secs_f64(),
    );
    crate::tracing_otel::spans::record_grpc_ok(&reload_span);
    // PR #17 — new `_seconds` histogram + (result, trigger) counter.
    // Audit-pass : map the threaded `trigger` string back to the
    // `ReloadTrigger` enum so the Prometheus label matches the OTel
    // span attribute.
    let metric_trigger = match trigger {
        "sighup" => crate::observability::metrics::ReloadTrigger::Sighup,
        "file_change" => crate::observability::metrics::ReloadTrigger::FileChange,
        _ => crate::observability::metrics::ReloadTrigger::Grpc,
    };
    state.metrics.manifest_reload().observe_duration(elapsed);
    state.metrics.manifest_reload().record_total(
        crate::observability::metrics::ReloadOutcome::Ok,
        metric_trigger,
    );
    // PR #17 AC6 — re-pretouch per-participant labels in case the
    // manifest's participants list changed.
    {
        let manifest_read = state.manifest.read().await;
        state.metrics.pretouch_for_manifest(&manifest_read);
    }

    info!(
        new_reload_gen,
        participants = participants_count,
        elapsed_us,
        changes = ?diff,
        "controller manifest reloaded",
    );

    Ok(ReloadOutcome {
        new_reload_gen,
        participants: participants_count,
        elapsed_us,
        applied_changes: diff,
    })
}

/// Produce a human-readable diff between two manifests.  Best-effort —
/// covers the common operator workflows (iteration knobs +
/// participants add/remove + safety geometry tweak) and surfaces them
/// in `ReloadConfigResponse.applied_changes` for grep-ability.
///
/// Returns an empty vector for a no-op reload.
fn diff_manifests(current: &Manifest, new: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    if (current.iteration.alpha_default - new.iteration.alpha_default).abs() > 1.0e-9 {
        out.push(format!(
            "iteration.alpha_default: {} → {}",
            current.iteration.alpha_default, new.iteration.alpha_default,
        ));
    }
    if (current.iteration.coupling_gamma - new.iteration.coupling_gamma).abs() > 1.0e-9 {
        out.push(format!(
            "iteration.coupling_gamma: {} → {}",
            current.iteration.coupling_gamma, new.iteration.coupling_gamma,
        ));
    }
    if current.iteration.max_iters != new.iteration.max_iters {
        out.push(format!(
            "iteration.max_iters: {} → {}",
            current.iteration.max_iters, new.iteration.max_iters,
        ));
    }
    if (current.iteration.tolerance - new.iteration.tolerance).abs() > 1.0e-12 {
        out.push(format!(
            "iteration.tolerance: {} → {}",
            current.iteration.tolerance, new.iteration.tolerance,
        ));
    }
    if current.iteration.max_wall_ms != new.iteration.max_wall_ms {
        out.push(format!(
            "iteration.max_wall_ms: {} → {}",
            current.iteration.max_wall_ms, new.iteration.max_wall_ms,
        ));
    }
    if current.iteration.deterministic != new.iteration.deterministic {
        out.push(format!(
            "iteration.deterministic: {} → {}",
            current.iteration.deterministic, new.iteration.deterministic,
        ));
    }

    // Safety params — kind is invariant (validate_reload_compatibility
    // refused mid-kind), but params inside a kind can drift.
    match (&current.meta.safety, &new.meta.safety) {
        (SafetyManifest::Box { radius: old_r }, SafetyManifest::Box { radius: new_r })
            if (old_r - new_r).abs() > 1.0e-6 =>
        {
            out.push(format!("meta.safety.radius: {old_r} → {new_r}"));
        }
        // Poly / Hull diffs are noisier ; surface a generic line.
        (SafetyManifest::Poly { .. }, SafetyManifest::Poly { .. }) => {
            // Cheap structural comparison ; deep equality on Vec<f32>
            // is meaningless for operators, so just flag any change.
            if !same_poly(&current.meta.safety, &new.meta.safety) {
                out.push("meta.safety.poly: parameters changed".to_string());
            }
        }
        (SafetyManifest::Hull { .. }, SafetyManifest::Hull { .. }) => {
            if !same_hull(&current.meta.safety, &new.meta.safety) {
                out.push("meta.safety.hull: parameters changed".to_string());
            }
        }
        _ => {
            // kind mismatch is impossible past validate_reload_compatibility.
        }
    }

    // Participants : added / removed / (no field drift — refused upstream).
    let old_ids: std::collections::HashSet<&str> =
        current.participants.iter().map(|p| p.id.as_str()).collect();
    let new_ids: std::collections::HashSet<&str> =
        new.participants.iter().map(|p| p.id.as_str()).collect();
    for added in new_ids.difference(&old_ids) {
        out.push(format!("participants: added {added}"));
    }
    for removed in old_ids.difference(&new_ids) {
        out.push(format!("participants: removed {removed}"));
    }

    out
}

fn same_poly(a: &SafetyManifest, b: &SafetyManifest) -> bool {
    if let (
        SafetyManifest::Poly {
            dim_a: da,
            a_flat: aa,
            b: ba,
        },
        SafetyManifest::Poly {
            dim_a: db,
            a_flat: ab,
            b: bb,
        },
    ) = (a, b)
    {
        da == db && aa == ab && ba == bb
    } else {
        false
    }
}

fn same_hull(a: &SafetyManifest, b: &SafetyManifest) -> bool {
    if let (
        SafetyManifest::Hull {
            version: va,
            serialized_model_path: pa,
        },
        SafetyManifest::Hull {
            version: vb,
            serialized_model_path: pb,
        },
    ) = (a, b)
    {
        va == vb && pa == pb
    } else {
        false
    }
}

/// Build a fresh `Manifest` from a `ReloadConfigRequest`'s inline
/// payload.  Used by `controller::service::reload_config` when the
/// request body is non-empty (AD11 inline-override path).
///
/// The inline payload only carries `new_meta` + `new_participants` ;
/// the `iteration` block is **inherited** from the current manifest
/// (the proto doesn't carry it).  This matches the operator's intuition
/// : "I'm changing the topology, not the iteration knobs."
///
/// # Errors
///
/// Returns [`ServerError::Manifest`] if any field can't be mapped
/// (unsupported safety geometry, etc.).
pub fn build_inline_manifest(
    req: &efe_meta_swarm_proto::meta_swarm::ReloadConfigRequest,
    current_iteration: crate::manifest::IterationManifest,
    current_version: String,
) -> Result<Manifest, ServerError> {
    use efe_meta_swarm_proto::meta_swarm::meta_space_config::SafetyGeometry as ProtoSafety;

    let new_meta = req.new_meta.as_ref().ok_or_else(|| {
        ServerError::Manifest(
            "build_inline_manifest : request.new_meta is required for inline override".into(),
        )
    })?;

    let safety = match new_meta.safety_geometry.as_ref() {
        Some(ProtoSafety::Box(b)) => SafetyManifest::Box { radius: b.radius },
        Some(ProtoSafety::Poly(p)) => SafetyManifest::Poly {
            dim_a: p.dim_a,
            a_flat: p.a_flat.clone(),
            b: p.b.clone(),
        },
        Some(ProtoSafety::Hull(h)) => SafetyManifest::Hull {
            // The proto only carries serialized bytes ; the manifest's
            // path-based schema is a Phase-2 placeholder anyway
            // (PR #14 will wire the real loader).  We stash a sentinel
            // here so callers see the kind, not the contents.
            version: "inline".into(),
            serialized_model_path: format!("<inline:{} bytes>", h.serialized_model.len(),),
        },
        None => {
            return Err(ServerError::Manifest(
                "build_inline_manifest : new_meta.safety_geometry is required".into(),
            ));
        }
    };

    let participants = req
        .new_participants
        .iter()
        .map(|p| ParticipantManifest {
            id: p.id.clone(),
            dim: p.dim,
            // The ReloadConfig RPC carries WIRE descriptors (mock-shaped) ;
            // it cannot express an efe-core `controller` block (M1.d) — and
            // engine changes across reload are rejected by
            // `validate_reload_compatibility` anyway.
            kappa: Some(p.kappa),
            seed: Some(p.seed),
            blas_backend: if p.blas_backend.is_empty() {
                "none".into()
            } else {
                p.blas_backend.clone()
            },
            controller: None,
        })
        .collect::<Vec<_>>();

    if participants.is_empty() {
        return Err(ServerError::Manifest(
            "build_inline_manifest : request.new_participants must not be empty".into(),
        ));
    }

    Ok(Manifest {
        version: current_version,
        meta: crate::manifest::MetaSpaceManifest {
            dim: new_meta.dim,
            safety,
        },
        participants,
        iteration: current_iteration,
    })
}

#[cfg(test)]
#[allow(clippy::module_name_repetitions)]
mod tests {
    use super::*;
    use crate::config::controller::{ControllerConfig, DiscoveryMode};
    use crate::controller::state::ControllerState;
    use crate::manifest::{
        IterationManifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
    };
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    fn three_p_manifest() -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 16,
                safety: SafetyManifest::Box { radius: 50.0 },
            },
            participants: vec![
                ParticipantManifest {
                    id: "supplier-scoring".into(),
                    dim: 8,
                    kappa: Some(0.6),
                    seed: Some(11),
                    blas_backend: "none".into(),
                    controller: None,
                },
                ParticipantManifest {
                    id: "contract-risk".into(),
                    dim: 8,
                    kappa: Some(0.7),
                    seed: Some(22),
                    blas_backend: "none".into(),
                    controller: None,
                },
                ParticipantManifest {
                    id: "spend-categorization".into(),
                    dim: 8,
                    kappa: Some(0.5),
                    seed: Some(33),
                    blas_backend: "none".into(),
                    controller: None,
                },
            ],
            iteration: IterationManifest::default(),
        }
    }

    fn test_state_with_manifest(manifest: Manifest, manifest_path: PathBuf) -> ControllerState {
        let config = ControllerConfig {
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            discovery: DiscoveryMode::StaticLocalhost,
            static_participants: vec![],
            manifest_path,
            max_inflight: 10,
            default_max_wall_ms: 15_000,
            worker_threads: 1,
            reflection_enabled: false,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off in reload-handler unit tests.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
            drain_timeout: Duration::from_secs(30),
        };
        ControllerState::new(config, manifest)
    }

    fn write_manifest_yaml(m: &Manifest) -> NamedTempFile {
        use std::io::Write;
        // serde_yaml round-trip keeps the schema compatible with
        // manifest::load_and_validate.
        let yaml = serde_yaml::to_string(m).expect("serialize manifest");
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(yaml.as_bytes()).expect("write");
        f.flush().expect("flush");
        f
    }

    #[tokio::test]
    async fn apply_reload_happy_path_bumps_gen_and_swaps_manifest() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        assert_eq!(state.health.snapshot().reload_gen, 0);

        let mut new = m0.clone();
        new.iteration.alpha_default = 0.7;
        new.iteration.coupling_gamma = 0.3;

        let outcome = apply_reload_with_manifest(&state, new.clone())
            .await
            .expect("happy path");
        assert_eq!(outcome.new_reload_gen, 1);
        assert_eq!(outcome.participants, 3);

        // Manifest field actually swapped.
        let post = state.manifest.read().await;
        assert!((post.iteration.alpha_default - 0.7).abs() < 1e-9);
        assert!((post.iteration.coupling_gamma - 0.3).abs() < 1e-9);

        // Health.reload_gen mirrors the outcome.
        assert_eq!(state.health.snapshot().reload_gen, 1);

        // applied_changes covers the two diffs (order is not asserted).
        assert!(outcome
            .applied_changes
            .iter()
            .any(|s| s.contains("alpha_default")));
        assert!(outcome
            .applied_changes
            .iter()
            .any(|s| s.contains("coupling_gamma")));
    }

    #[tokio::test]
    async fn apply_reload_rejects_dim_change_for_existing_participant() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.participants[0].dim = 16; // illegal drift
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("dim drift")));
        // State untouched : reload_gen still 0 (transactional, AD6).
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn apply_reload_rejects_kappa_change_for_existing_participant() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.participants[1].kappa = Some(0.85);
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("kappa drift")));
        // Manifest unchanged.
        let post = state.manifest.read().await;
        assert!((post.participants[1].kappa.unwrap_or(f32::NAN) - 0.7).abs() < 1e-6);
    }

    #[tokio::test]
    async fn apply_reload_rejects_meta_dim_change() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.meta.dim = 32;
        let err = apply_reload_with_manifest(&state, new).await.unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref msg) if msg.contains("meta.dim")));
    }

    #[tokio::test]
    async fn apply_reload_accepts_added_participant() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.participants.push(ParticipantManifest {
            id: "fraud-detection".into(),
            dim: 8,
            kappa: Some(0.55),
            seed: Some(44),
            blas_backend: "none".into(),
            controller: None,
        });
        let outcome = apply_reload_with_manifest(&state, new).await.expect("ok");
        assert_eq!(outcome.participants, 4);
        // applied_changes records the addition.
        assert!(outcome
            .applied_changes
            .iter()
            .any(|s| s == "participants: added fraud-detection"));
    }

    #[tokio::test]
    async fn apply_reload_accepts_removed_participant() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.participants.remove(2);
        let outcome = apply_reload_with_manifest(&state, new).await.expect("ok");
        assert_eq!(outcome.participants, 2);
        assert!(outcome
            .applied_changes
            .iter()
            .any(|s| s == "participants: removed spend-categorization"));
    }

    #[tokio::test]
    async fn apply_reload_failed_validation_leaves_state_untouched() {
        // Bad manifest : kappa out of range.  Top-level validate_manifest
        // rejects ; reload_gen must remain 0 and state.manifest must
        // still be the original.
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut bad = m0.clone();
        bad.participants[0].kappa = Some(1.5); // outside (0, 1)
        let err = apply_reload_with_manifest(&state, bad).await.unwrap_err();
        assert!(matches!(err, ServerError::Manifest(_)));
        assert_eq!(state.health.snapshot().reload_gen, 0);
        let post = state.manifest.read().await;
        assert!((post.participants[0].kappa.unwrap_or(f32::NAN) - 0.6).abs() < 1e-6);
    }

    #[tokio::test]
    async fn apply_reload_concurrent_iterate_sees_consistent_snapshot() {
        // The load-bearing invariant : a clone of `Arc<Manifest>`
        // captured BEFORE the swap keeps the old participants visible
        // forever.  This is the same pattern PR #8 uses in
        // `run_iterate` (which does `state.manifest.read().await.clone()`).
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());

        // "Concurrent iterate" : capture the clone NOW.
        let snapshot_before = state.manifest.read().await.clone();
        assert_eq!(snapshot_before.participants.len(), 3);

        // Reload : remove a participant.
        let mut new = m0.clone();
        new.participants.pop();
        let _ = apply_reload_with_manifest(&state, new).await.expect("ok");

        // The snapshot from BEFORE still has 3.  In-flight Iterates
        // see this.
        assert_eq!(
            snapshot_before.participants.len(),
            3,
            "pre-reload clone must remain intact (architecture § 5.2)",
        );

        // The post-reload read sees 2.
        let post = state.manifest.read().await;
        assert_eq!(post.participants.len(), 2);
    }

    #[tokio::test]
    async fn apply_reload_from_disk_loads_new_manifest_and_bumps_gen() {
        let m0 = three_p_manifest();
        // The original on-disk file keeps the tempfile alive for the
        // baseline (unused but its `Drop` removes the file if we never
        // pointed the state at it).
        let _f0 = write_manifest_yaml(&m0);
        // Mutate the on-disk file : bump alpha to 0.75.
        let mut new = m0.clone();
        new.iteration.alpha_default = 0.75;
        let f2 = write_manifest_yaml(&new);
        let state = test_state_with_manifest(m0.clone(), f2.path().to_path_buf());

        let outcome = apply_reload_from_disk(&state).await.expect("disk reload");
        assert_eq!(outcome.new_reload_gen, 1);

        let post = state.manifest.read().await;
        assert!((post.iteration.alpha_default - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn apply_reload_from_disk_propagates_io_error() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0, "/nonexistent/path/manifest.yaml".into());
        let err = apply_reload_from_disk(&state).await.unwrap_err();
        assert!(matches!(err, ServerError::Io(_)));
        // failure metric counter incremented under the io label.
        let body = String::from_utf8(state.metrics.render()).unwrap();
        assert!(body.contains("efe_meta_swarm_reload_failures_total{reason=\"io\"}"));
    }

    #[tokio::test]
    async fn apply_reload_records_metrics_on_success() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.iteration.alpha_default = 0.65;
        let _ = apply_reload_with_manifest(&state, new).await.expect("ok");
        // attempts_total +1, reload_gen gauge == 1, duration histogram
        // recorded one sample.
        let body = String::from_utf8(state.metrics.render()).unwrap();
        assert!(body.contains("efe_meta_swarm_reload_attempts_total 1"));
        assert!(body.contains("efe_meta_swarm_reload_gen 1"));
        // Sample count == 1 in the histogram.
        assert!(body.contains("efe_meta_swarm_reload_duration_microseconds_count 1"));
    }

    #[tokio::test]
    async fn apply_reload_records_metrics_on_drift_failure() {
        let m0 = three_p_manifest();
        let state = test_state_with_manifest(m0.clone(), "/dev/null".into());
        let mut new = m0.clone();
        new.participants[0].dim = 99;
        let _ = apply_reload_with_manifest(&state, new).await.unwrap_err();
        let body = String::from_utf8(state.metrics.render()).unwrap();
        assert!(body.contains("efe_meta_swarm_reload_attempts_total 1"));
        assert!(body.contains("efe_meta_swarm_reload_failures_total{reason=\"drift\"} 1"));
        // gen gauge must NOT advance.
        assert!(body.contains("efe_meta_swarm_reload_gen 0"));
    }

    // -----------------------------------------------------------------
    // build_inline_manifest tests
    // -----------------------------------------------------------------

    fn inline_request(
        meta_dim: u32,
        radius: f32,
        rows: &[(&str, u32, f32, u64)],
    ) -> efe_meta_swarm_proto::meta_swarm::ReloadConfigRequest {
        use efe_meta_swarm_proto::meta_swarm::{
            meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, MetaSpaceConfig,
            ParticipantDescriptor, ReloadConfigRequest,
        };
        ReloadConfigRequest {
            new_meta: Some(MetaSpaceConfig {
                dim: meta_dim,
                safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius })),
            }),
            new_participants: rows
                .iter()
                .map(|(id, dim, kappa, seed)| ParticipantDescriptor {
                    id: (*id).into(),
                    dim: *dim,
                    kappa: *kappa,
                    seed: *seed,
                    build_sha: String::new(),
                    blas_backend: String::new(),
                    features: vec![],
                    config_hash: None,
                })
                .collect(),
            drain_inflight: false,
        }
    }

    #[test]
    fn build_inline_manifest_happy_path() {
        let req = inline_request(
            16,
            42.0,
            &[("a", 8, 0.6, 11), ("b", 8, 0.7, 22), ("c", 8, 0.5, 33)],
        );
        let cur_iter = IterationManifest::default();
        let m = build_inline_manifest(&req, cur_iter.clone(), "v0.2.0".into()).expect("inline ok");
        assert_eq!(m.version, "v0.2.0");
        assert_eq!(m.meta.dim, 16);
        match m.meta.safety {
            SafetyManifest::Box { radius } => assert!((radius - 42.0).abs() < 1e-6),
            _ => panic!("expected Box"),
        }
        assert_eq!(m.participants.len(), 3);
        assert_eq!(m.participants[0].id, "a");
        // Iteration block inherited intact.
        assert!((m.iteration.alpha_default - cur_iter.alpha_default).abs() < 1e-12);
    }

    #[test]
    fn build_inline_manifest_rejects_empty_participants() {
        let req = inline_request(16, 5.0, &[]);
        let err =
            build_inline_manifest(&req, IterationManifest::default(), "v0.2.0".into()).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref m) if m.contains("must not be empty")));
    }

    #[test]
    fn build_inline_manifest_rejects_missing_safety() {
        use efe_meta_swarm_proto::meta_swarm::{
            MetaSpaceConfig, ParticipantDescriptor, ReloadConfigRequest,
        };
        let req = ReloadConfigRequest {
            new_meta: Some(MetaSpaceConfig {
                dim: 16,
                safety_geometry: None,
            }),
            new_participants: vec![ParticipantDescriptor {
                id: "a".into(),
                dim: 8,
                kappa: 0.5,
                seed: 1,
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            }],
            drain_inflight: false,
        };
        let err =
            build_inline_manifest(&req, IterationManifest::default(), "v0.2.0".into()).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref m) if m.contains("safety_geometry")));
    }

    #[test]
    fn build_inline_manifest_rejects_missing_new_meta() {
        use efe_meta_swarm_proto::meta_swarm::{ParticipantDescriptor, ReloadConfigRequest};
        let req = ReloadConfigRequest {
            new_meta: None,
            new_participants: vec![ParticipantDescriptor {
                id: "a".into(),
                dim: 8,
                kappa: 0.5,
                seed: 1,
                build_sha: String::new(),
                blas_backend: String::new(),
                features: vec![],
                config_hash: None,
            }],
            drain_inflight: false,
        };
        let err =
            build_inline_manifest(&req, IterationManifest::default(), "v0.2.0".into()).unwrap_err();
        assert!(matches!(err, ServerError::Manifest(ref m) if m.contains("new_meta is required")));
    }
}
