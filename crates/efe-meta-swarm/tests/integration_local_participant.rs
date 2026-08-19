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

use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmId, SwarmSpec, TemplateCode};
use efe_meta_swarm::participants::{LocalParticipant, SwarmParticipant};
use efe_meta_swarm::safety::{BoxSafety, SafetySet};
use efe_meta_swarm::Tensor;
use efe_server::mode1::{build_mvp_controller, build_mvp_controller_with_init_sigma, Mode1Config};

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

// =====================================================================
// Couche 3 · M1.b — Φ is DETERMINISTIC (bit-reproducible)
// =====================================================================

/// Bit-for-bit equality of two latent vectors, compared on the raw IEEE-754 bit
/// patterns (so `+0.0` vs `-0.0`, and any 1-ULP drift, are caught — plain `==` on
/// `f32` treats `+0.0 == -0.0`). `to_bits()` equality means two *identical* NaN bit
/// patterns compare equal — exactly what a bit-reproducibility check wants (the
/// outputs are finite, asserted separately by the M1.a finiteness test).
fn bit_identical(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Φ has **no per-call nondeterminism**: repeated `phi_once(x)` on one controller
/// with one fixed spec return the bit-identical latent. This rules out a stray
/// `thread_rng`, wall-clock dependence, or hash-iteration-order effect on the Φ path.
///
/// NOTE — this is *necessary but not sufficient* to validate the M1.b lever. The
/// genesis noise seed is `compute_seed(spec.id)` (`spawner.rs`), fixed for a fixed
/// spec, so Φ is bit-reproducible here at **any** `init_sigma` (including the 0.02
/// default). The property that actually pins `init_sigma = 0` is *seed-invariance*
/// — see `real_phi_once_is_noise_seed_invariant`, which varies `spec.id` and so
/// FAILS if the noise is ever re-enabled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_phi_once_is_bit_deterministic() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let spec = base_spec();
    let x = vec![0.137f32; DIM];
    let a = controller.phi_once(&x, &spec, "tenant-m1b").await.expect("phi_once #1");
    let b = controller.phi_once(&x, &spec, "tenant-m1b").await.expect("phi_once #2");
    assert_eq!(a.len(), DIM, "Φ(x) has the latent dimension");
    assert!(
        bit_identical(&a, &b),
        "Φ(x) MUST be bit-identical across repeated calls (no per-call entropy).\n a={a:?}\n b={b:?}"
    );
}

/// **The M1.b lever, with teeth.** The genesis noise seed is `compute_seed(spec.id)`
/// (`spawner.rs`; `GenesisEngine::spawn` passes `spec.id`, *not* the fresh telemetry
/// `SwarmId`), so `spec.id` is the sole entropy source on the Φ path. We hold `x`
/// fixed and feed **two specs with distinct ids**: with `init_sigma = 0` (the latent
/// controller) there is no noise draw, so Φ(x) is bit-identical; with the server
/// default `init_sigma = 0.02` the two ids seed **different** genesis noise and Φ(x)
/// would differ. So this test **fails if the noise lever is ever re-enabled** — it is
/// the guard the M1.c Lipschitz gate stands on. (A fixed noise realization ε perturbs
/// the nonlinear pipeline and does not cancel in Φ(x)−Φ(y); `init_sigma = 0` removes ε
/// so the measured Lipschitz ratio reflects x alone.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_phi_once_is_noise_seed_invariant() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let mut spec_a = base_spec();
    let mut spec_b = base_spec();
    spec_a.id = SwarmId::new();
    spec_b.id = SwarmId::new();
    assert_ne!(
        spec_a.id, spec_b.id,
        "test precondition: the two specs must carry DIFFERENT ids (⇒ different noise seeds)"
    );
    let x = vec![0.137f32; DIM];
    let a = controller.phi_once(&x, &spec_a, "tenant-m1b").await.expect("Φ(x) @ id_a");
    let b = controller.phi_once(&x, &spec_b, "tenant-m1b").await.expect("Φ(x) @ id_b");
    assert!(
        bit_identical(&a, &b),
        "Φ(x) MUST be invariant to the genesis noise seed (spec.id) — holds only at init_sigma=0.\n a={a:?}\n b={b:?}"
    );
}

/// Φ(x) is **invariant to call order** — no mutable state leaks across
/// **sequential** `execute()` calls. We interleave Φ(x), Φ(y), Φ(x): the first
/// and third must be bit-identical. This is the decisive guard against a stale
/// `LastCentroid` channel, a template/meta cache, or reused-arena residue
/// carrying data from the intervening Φ(y) into the second Φ(x).
///
/// `y` is a *different but in-domain* input: Φ is only defined where the fused
/// centroid satisfies the participant's alignment cone (base_spec's cone points
/// along `+[0.1; DIM]`). An anti-aligned input (e.g. all-negative) is rejected
/// with `AlignmentBreach` — that is Φ's **domain boundary**, not
/// non-determinism, and the M1.c Lipschitz canary must sample inside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_phi_once_is_call_order_invariant() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let spec = base_spec();
    let x = vec![0.2f32; DIM];
    let y = vec![0.08f32; DIM]; // in-domain (uniform positive, aligned with the cone)
    let x1 = controller.phi_once(&x, &spec, "tenant-m1b").await.expect("Φ(x) #1");
    let _y = controller.phi_once(&y, &spec, "tenant-m1b").await.expect("Φ(y)");
    let x2 = controller.phi_once(&x, &spec, "tenant-m1b").await.expect("Φ(x) #2");
    assert!(
        bit_identical(&x1, &x2),
        "Φ(x) after an intervening Φ(y) must equal Φ(x) — no cross-call state leak.\n x1={x1:?}\n x2={x2:?}"
    );
}

/// Two **independently constructed** latent controllers, each driven with a spec
/// carrying a **distinct** `spec.id`, compute the bit-identical Φ(x). This layers two
/// guarantees at once: controller *construction* carries no entropy (no
/// process-global, no per-controller randomness), AND the operator is
/// seed-invariant (distinct ids ⇒ distinct noise seeds, which agree only once
/// `init_sigma = 0` disables the noise). Like `noise_seed_invariant`, it fails if the
/// noise lever is re-enabled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_phi_once_is_deterministic_across_controllers() {
    let c1 = build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("controller 1");
    let c2 = build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("controller 2");
    let mut spec1 = base_spec();
    let mut spec2 = base_spec();
    spec1.id = SwarmId::new();
    spec2.id = SwarmId::new();
    assert_ne!(spec1.id, spec2.id, "test precondition: distinct spec ids ⇒ distinct noise seeds");
    let x = vec![0.05f32; DIM];
    let a = c1.phi_once(&x, &spec1, "tenant-m1b").await.expect("Φ on c1");
    let b = c2.phi_once(&x, &spec2, "tenant-m1b").await.expect("Φ on c2");
    assert!(
        bit_identical(&a, &b),
        "two independently built latent controllers with distinct seeds must compute the same Φ(x).\n a={a:?}\n b={b:?}"
    );
}

/// **Negative control — the M1.b lever has teeth.** The dual of
/// `real_phi_once_is_noise_seed_invariant`: built with the server's *actual* default
/// genesis noise (`init_sigma = 0.02`, via the doc-hidden test hatch), Φ(x) becomes
/// SEED-DEPENDENT — two specs with distinct `spec.id` seed different genesis noise, so
/// Φ(x) differs (empirically measured ‖Δ‖₂ ≈ 7·10⁻⁴ ≠ 0 at this sigma — the noise
/// survives the bf16 genesis quantization, so it is a *real* per-seed difference, not a
/// rounding artifact). Together the two tests pin `init_sigma` as exactly the
/// determinism lever: a regression that stopped zeroing the noise would make Φ
/// seed-dependent and break `noise_seed_invariant` + `across_controllers`; a regression
/// that broke the noise path would break THIS test. Neither can silently pass green.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn negative_control_noisy_phi_is_seed_dependent() {
    // `build_mvp_controller_with_init_sigma` is a #[doc(hidden)] TEST hatch — production
    // uses `build_mvp_controller` (init_sigma=0). 0.02 is the server Mode-1 default noise.
    let controller = build_mvp_controller_with_init_sigma(Mode1Config::for_dim(DIM as u32), 0.02)
        .expect("build noisy control controller");
    let mut spec_a = base_spec();
    let mut spec_b = base_spec();
    spec_a.id = SwarmId::new();
    spec_b.id = SwarmId::new();
    assert_ne!(spec_a.id, spec_b.id, "test precondition: distinct spec ids ⇒ distinct noise seeds");
    let x = vec![0.137f32; DIM];
    let a = controller.phi_once(&x, &spec_a, "tenant-m1b").await.expect("Φ(x) @ id_a (noisy)");
    let b = controller.phi_once(&x, &spec_b, "tenant-m1b").await.expect("Φ(x) @ id_b (noisy)");
    assert!(
        !bit_identical(&a, &b),
        "with genesis noise ON (init_sigma=0.02) Φ(x) MUST vary with the seed (spec.id) — exactly what init_sigma=0 removes.\n a={a:?}\n b={b:?}"
    );
}

/// The full coupled Krasnosel'skii-Mann step is deterministic too: `step_with_pull`
/// = `mix_coupled(x, Φ(x), pull, α, γ)`, and `mix_coupled` is a pure affine
/// combination, so a deterministic Φ makes the whole step bit-reproducible.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_step_with_pull_is_deterministic() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real SwarmController");
    let spec = base_spec();
    let x = vec![0.3f32; DIM];
    let pull = vec![0.1f32; DIM];
    let (alpha, gamma) = (0.2f64, 0.1f64);
    let a = controller
        .step_with_pull(&x, &pull, alpha, gamma, &spec, "tenant-m1b")
        .await
        .expect("step #1");
    let b = controller
        .step_with_pull(&x, &pull, alpha, gamma, &spec, "tenant-m1b")
        .await
        .expect("step #2");
    assert!(
        bit_identical(&a, &b),
        "the coupled KM step must be bit-identical across calls.\n a={a:?}\n b={b:?}"
    );
}
