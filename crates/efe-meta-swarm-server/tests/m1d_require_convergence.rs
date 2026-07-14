// SPDX-License-Identifier: Apache-2.0
//! **Couche 3 · M1.d — fail-closed non-convergence on the served Iterate.**
//!
//! Architecture §4 : `run()` rendait `Ok(MaxItersHit)` — un RPC « réussi »
//! portant un « consensus » **non convergé** que le client peut consommer en
//! silence.  Le manifeste porte désormais `iteration.require_convergence` :
//!
//! * `true`  → un Iterate qui s'arrête sur `MaxItersHit` rend **gRPC
//!   `ABORTED`** (avec iterations + residual dans le message) — jamais un
//!   consensus.
//! * `false` (défaut, rétro-compat v0.2.0) → comportement historique : OK +
//!   `stop_reason = MAX_ITERS`, au client d'inspecter.
//!
//! Les deux moitiés sont épinglées ici sur le VRAI stack gRPC in-proc
//! (contrôleur + 3 participants mock), plus la preuve que le gate ne tire
//! JAMAIS sur une itération qui converge réellement.

mod common;

use efe_meta_swarm_proto::meta_swarm::{StepOnceRequest, StopReason as ProtoStopReason};
use efe_meta_swarm_server::controller::ControllerService;
use efe_meta_swarm_server::manifest::IterationManifest;
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::{Code, Request};

const DIM: usize = 8;

async fn spawn_three() -> Vec<(String, usize, f32, u64, std::net::SocketAddr)> {
    let (a1, _h1) = common::spawn_participant_simple("supplier-scoring", DIM as u32, 0.6, 11).await;
    let (a2, _h2) = common::spawn_participant_simple("contract-risk", DIM as u32, 0.7, 22).await;
    let (a3, _h3) =
        common::spawn_participant_simple("spend-categorization", DIM as u32, 0.5, 33).await;
    // NOTE : the JoinHandles are detached — the participant tasks live for
    // the test process lifetime, matching the existing in-proc smoke tests.
    vec![
        ("supplier-scoring".into(), DIM, 0.6_f32, 11_u64, a1),
        ("contract-risk".into(), DIM, 0.7_f32, 22_u64, a2),
        ("spend-categorization".into(), DIM, 0.5_f32, 33_u64, a3),
    ]
}

fn iteration(require_convergence: bool) -> IterationManifest {
    IterationManifest {
        max_iters: 100,
        tolerance: 1.0e-4,
        alpha_default: 0.5,
        coupling_gamma: 0.2,
        deterministic: true,
        max_wall_ms: 15_000,
        require_convergence,
    }
}

/// require_convergence = TRUE + an unreachable tolerance ⇒ the Iterate RPC
/// FAILS with `ABORTED` and a diagnostic message — never an OK response.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strict_profile_aborts_on_non_convergence() {
    let statics = spawn_three().await;
    let state = common::build_controller_state_with_iteration(&statics, 10, DIM, iteration(true));
    let svc = ControllerService::new(state);

    // 2 iterations at tolerance 1e-12 cannot converge (κ ≥ 0.5 ⇒ ~50+ iters).
    let err = svc
        .iterate(Request::new(common::iterate_request(DIM as u32, 2, 1.0e-12)))
        .await
        .expect_err("a non-converged Iterate must FAIL when require_convergence is set");
    assert_eq!(err.code(), Code::Aborted, "fail-closed code is ABORTED, got {err:?}");
    let msg = err.message();
    assert!(
        msg.contains("convergence failed") && msg.contains("iterations"),
        "the error must carry the diagnostics, got : {msg}",
    );
}

/// require_convergence = FALSE (the default) keeps the historical lenient
/// behaviour : OK + `stop_reason = MAX_ITERS` — backward compat pinned.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lenient_default_returns_ok_with_max_iters() {
    let statics = spawn_three().await;
    let state = common::build_controller_state_with_iteration(&statics, 10, DIM, iteration(false));
    let svc = ControllerService::new(state);

    let resp = svc
        .iterate(Request::new(common::iterate_request(DIM as u32, 2, 1.0e-12)))
        .await
        .expect("lenient mode returns OK even without convergence")
        .into_inner();
    assert_eq!(
        resp.stop_reason,
        ProtoStopReason::MaxIters as i32,
        "the honest stop_reason is still on the wire : {resp:?}",
    );
}

/// The gate NEVER fires on genuine convergence : strict profile + a reachable
/// tolerance ⇒ OK + Converged (the regression net for the four existing
/// convergence e2e tests).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strict_profile_passes_a_converging_iterate() {
    let statics = spawn_three().await;
    let state = common::build_controller_state_with_iteration(&statics, 10, DIM, iteration(true));
    let svc = ControllerService::new(state);

    let resp = svc
        .iterate(Request::new(common::iterate_request(DIM as u32, 100, 1.0e-3)))
        .await
        .expect("a converging Iterate passes the strict profile")
        .into_inner();
    assert_eq!(resp.stop_reason, ProtoStopReason::Converged as i32, "{resp:?}");
}

/// **`StepOnce` is EXEMPT from `require_convergence`** (by design — see
/// `controller::iterate::run_step_once`).  A single Φ step has no convergence
/// verdict ; gating it on the whole-Iterate flag would abort EVERY `StepOnce`
/// under the strict profile and brick the primitive.  Same strict profile that
/// ABORTS a non-converged `Iterate` above ⇒ `StepOnce` still returns OK.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn step_once_is_exempt_from_require_convergence() {
    let statics = spawn_three().await;
    let state = common::build_controller_state_with_iteration(&statics, 10, DIM, iteration(true));
    let svc = ControllerService::new(state);

    let resp = svc
        .step_once(Request::new(StepOnceRequest::default()))
        .await
        .expect("StepOnce is exempt from require_convergence — it must NOT abort")
        .into_inner();
    // The single-step primitive reports a finite residual and carries NO
    // stop_reason field (there is no convergence verdict for one step).
    assert!(
        resp.residual.is_finite(),
        "StepOnce returns a finite residual : {resp:?}",
    );
}
