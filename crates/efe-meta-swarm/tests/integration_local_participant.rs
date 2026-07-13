// SPDX-License-Identifier: UNLICENSED
//! **Couche 3 · M1.a — the real efe-core participant bridge, exercised end-to-end.**
//!
//! Proves the SEAM works with a **real** `efe_core::SwarmController` (not the
//! `MockParticipant`): build a Mode-1 controller → wrap it in the
//! `core-integration` [`LocalParticipant`] → drive the isolated operator
//! `phi_once` and the full coupled `step_with_pull`.  The whole pipeline
//! (genesis → fusion → dissolve → align → decode → `mix_coupled` → safety
//! projection) runs, stays finite, and is contained in the participant's
//! safety box.  This is the file `participants/local.rs:264` promised but that
//! never existed — the bridge had never run with a real controller.
//!
//! Gated by the `integration-test` feature (**Voie A**) so the heavy
//! `efe-server` crate (which owns `mode1::build_mvp_controller`) is pulled in
//! **only** here — never in the default build nor the production
//! `core-integration` path.  Run with:
//!
//! ```text
//! cargo test -p efe-meta-swarm --features integration-test
//! ```
//!
//! ## Honesty (Couche 3 — no over-sell)
//!
//! - The participant runs efe-core's **numerical** Mode-1 fusion on **stub
//!   templates** — real efe-core *latent* swarms, **not** language-model SLMs
//!   (no model weights / tokenizer on the step path).
//! - `Φ` is **not yet deterministic** (each `execute()` draws a fresh
//!   `SwarmId`, seeding genesis noise — M1.d).  This test therefore does
//!   **not** assert bit-identical reproducibility.
//! - Containment is *guaranteed by construction* (the participant's
//!   `BoxSafety` projection clamps every state) — the test's real assertion is
//!   that the whole real pipeline **runs without error/NaN and does not blow
//!   up** (a soft contraction signal), which is exactly what "wire a real
//!   participant" must prove before any Kovant demo.

#![cfg(feature = "integration-test")]

use std::sync::Arc;

use tokio::runtime::Handle;

use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_meta_swarm::participants::{LocalParticipant, SwarmParticipant};
use efe_meta_swarm::safety::{BoxSafety, SafetySet};
use efe_meta_swarm::Tensor;
use efe_server::mode1::{build_mvp_controller, Mode1Config};

/// Small latent dimension — the Mode-1 stub registry / meta-analyzer are
/// dim-parameterised (`GenesisEngine::build_mvp` → `mvp_defaults(dim)`,
/// `HeuristicMetaAnalyzer::load_default(dim)`), so a small dim keeps the test
/// fast without hitting the 384-d embedding defaults.
const DIM: usize = 16;
const BOX_RADIUS: f32 = 10.0;

/// A minimal `SwarmSpec` compatible with `Mode1Config::for_dim(DIM)`'s stub
/// registry (`TemplateCode::NaSem` ∈ `TemplateCode::MVP_SET`).  `phi_once` /
/// `step_with_pull` override `problem_embedding` + `max_rounds` per call, so
/// only the *shape* matters here.
fn base_spec() -> SwarmSpec {
    SwarmSpec::builder()
        .problem_embedding(vec![0.0; DIM])
        .dim_latent(LatentDim::new(DIM as u32).expect("valid dim"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid population"))
        .max_rounds(3)
        .expect("valid rounds")
        .alignment_cone(AlignmentCone::new(vec![vec![0.1; DIM]], 0.5).expect("valid cone"))
        .build()
        .expect("valid spec")
}

fn real_participant() -> LocalParticipant {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let safety: Box<dyn SafetySet> = Box::new(BoxSafety::symmetric(DIM, BOX_RADIUS));
    LocalParticipant::new(
        "p0",
        controller,
        base_spec(),
        "tenant-m1a",
        safety,
        Handle::current(),
    )
    .expect("build LocalParticipant over the real controller")
}

/// The **isolated** operator `Φ_m(x)` runs on a real controller and returns a
/// finite, dim-correct latent — the foundation the M1.c empirical-Lipschitz
/// canary measures (Φ without the α/γ coupling).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_phi_once_is_finite_and_dim_correct() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let x = vec![0.1f32; DIM];
    let spec = base_spec();
    // `phi_once` is a plain async fn (no internal block_on) → await directly.
    let phi = controller
        .phi_once(&x, &spec, "tenant-m1a")
        .await
        .expect("phi_once runs one real fusion round");
    assert_eq!(phi.len(), DIM, "Φ(x) has the latent dimension");
    assert!(phi.iter().all(|v| v.is_finite()), "Φ(x) is finite (no NaN from the real fusion)");
}

/// The full coupled step runs end-to-end on a real controller and stays
/// contained in the participant's safety box, with no divergence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_participant_steps_run_and_stay_contained() {
    let p = Arc::new(real_participant());
    p.set_state(Tensor::from_vec(vec![0.3f32; DIM])).expect("seed state");

    let pull = Tensor::from_vec(vec![0.0f32; DIM]);
    let (alpha, gamma) = (0.2f64, 0.1f64);

    let mut prev = p.current_state().expect("read state").as_slice().to_vec();
    for step in 0..5u32 {
        let pc = p.clone();
        let pull_c = pull.clone();
        // The participant bridges sync→async via `runtime.block_on`, which must
        // NOT run on a Tokio worker → drive it on a blocking thread.
        let next = tokio::task::spawn_blocking(move || pc.step_with_pull(&pull_c, gamma, alpha))
            .await
            .expect("join blocking step")
            .expect("real coupled step");

        // 1. shape + finiteness — the real numerical fusion produced a valid latent.
        assert_eq!(next.dim(), DIM, "step {step}: dim");
        assert!(
            next.as_slice().iter().all(|v| v.is_finite()),
            "step {step}: finite (no NaN propagated from genesis/fusion/align)"
        );
        // 2. containment — the participant's BoxSafety projection is enforced.
        assert!(
            next.as_slice().iter().all(|v| v.abs() <= BOX_RADIUS + 1e-3),
            "step {step}: contained in the ±{BOX_RADIUS} safety box"
        );
        // 3. no blow-up — successive change stays bounded (soft contraction
        //    signal ; NOT asserted monotone because Φ is not yet deterministic, M1.d).
        let diff: f32 = next
            .as_slice()
            .iter()
            .zip(&prev)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            diff.is_finite() && diff <= 4.0 * BOX_RADIUS,
            "step {step}: successive change bounded (‖Δ‖={diff})"
        );
        prev = next.as_slice().to_vec();
    }
}
