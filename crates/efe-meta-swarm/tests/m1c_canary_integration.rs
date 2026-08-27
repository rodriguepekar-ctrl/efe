// SPDX-License-Identifier: UNLICENSED
//! **Couche 3 · M1.c — the empirical Lipschitz canary, over a REAL efe-core Φ.**
//!
//! The payoff of M1.c: build a real `LocalParticipant`, run its
//! [`SwarmParticipant::verify_non_expansive`] — which drives the adversarial Lipschitz
//! canary ([`efe_meta_swarm::participants::lipschitz`]) over the participant's
//! operational domain using `phi_once` — and read the resulting `Empirical`
//! certificate. This **empirically probes** the architecture's central claim (§1): the
//! efe-core one-round `Φ` is an **intra-cell `~β₁ ≈ 0.17` contraction**, i.e. H1
//! (`L_Φ ≤ 1`) holds on a within-cell domain.
//!
//! Gated by `integration-test` (Voie A) — pulls `efe-mode1` only for the test (R1 : une feuille, plus la pile applicative).
//!
//! ## Honesty (no over-sell)
//!
//! `L̂` is a **lower bound** — a pass is "non-expansiveness property-tested, NOT
//! certified" (see the module docs). The domain here is a small positive box chosen to
//! sit inside `Φ`'s alignment cone; the canary's `phi_once` calls are driven
//! **single-flight** (`verify_non_expansive` blocks each to completion), honouring the
//! shared-`LastCentroid` constraint.

#![cfg(feature = "integration-test")]

use std::sync::Arc;

use tokio::runtime::Handle;

use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_meta_swarm::participants::{
    LipschitzProbe, LocalParticipant, NonExpansiveCertificate, SwarmParticipant,
};
use efe_meta_swarm::safety::{BoxSafety, SafetySet};
use efe_mode1::mode1::{build_mvp_controller, Mode1Config};

const DIM: usize = 16;

/// A minimal spec compatible with `Mode1Config::for_dim(DIM)`; the alignment cone points
/// along `+[0.1; DIM]`, so a small POSITIVE input box is in-domain. `phi_once` overrides
/// `problem_embedding` + `max_rounds`, so only the shape matters.
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

/// Run the real canary over a within-cell positive domain and assert Φ is non-expansive
/// there (H1 property-tested), reporting the measured `L̂`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_lipschitz_canary_measures_phi_as_a_contraction() {
    let controller =
        build_mvp_controller(Mode1Config::for_dim(DIM as u32)).expect("build real controller");

    // Operational domain = a small POSITIVE box, inside Φ's alignment cone (`+[0.1;DIM]`)
    // and within the range M1.b showed to be in-domain (0.05..0.3). An anti-aligned point
    // would `AlignmentBreach` (out-of-domain) — the canary skips those.
    let safety: Box<dyn SafetySet> =
        Box::new(BoxSafety::new(vec![0.05_f32; DIM], vec![0.30_f32; DIM]).expect("valid box"));

    // A small probe keeps the real-Φ canary to ~100 `phi_once` calls (each ~tens of ms).
    let probe = {
        let mut p = LipschitzProbe::small();
        p.n_seeds = 16;
        p.n_directions = 3;
        p.n_bisect_pairs = 4;
        p.bisect_depth = 16;
        p
    };

    let participant = LocalParticipant::new(
        "p-canary",
        controller,
        base_spec(),
        "tenant-m1c",
        safety,
        Handle::current(),
    )
    .expect("build LocalParticipant")
    .with_lipschitz_probe(probe);

    // `verify_non_expansive` bridges sync→async via `block_on` internally → it must NOT
    // run on a Tokio worker; drive it on a blocking thread.
    let cert = tokio::task::spawn_blocking(move || participant.verify_non_expansive())
        .await
        .expect("join blocking canary")
        .expect("the canary ran without a fatal error");

    let accepts = cert.accepts(false);
    match cert {
        NonExpansiveCertificate::Empirical {
            l_hat,
            margin,
            n_pairs_scored,
            n_domain_errors,
            inconclusive,
        } => {
            eprintln!(
                "M1.c canary on real efe-core Φ: L̂={l_hat:.5}  margin={margin}  \
                 pairs_scored={n_pairs_scored}  domain_errors={n_domain_errors}  \
                 inconclusive={inconclusive}  accepts={accepts}"
            );
            assert!(!inconclusive, "the positive within-cone box must yield ≥2 in-domain samples");
            assert!(n_pairs_scored > 0, "the canary must score at least one pair");
            assert!(l_hat.is_finite(), "L̂ must be finite (no NaN/Inf from the real fusion)");
            // Architecture §1 predicted an intra-cell `~β₁≈0.17` contraction from the
            // beta_mix weight alone. The MEASURED composite bound (whole pipeline +
            // routing sensitivity) is higher — empirically ≈0.69 on this box — but still
            // `< 1`, so H1 holds here (property-tested, not certified). That gap (0.69 vs
            // the analytic 0.17) is exactly why we MEASURE rather than trust the optimistic
            // closed form: Φ is non-expansive, but with less margin than assumed, and a
            // domain that straddled a routing boundary could push it past 1 (which the
            // adversarial bisection would then expose).
            assert!(
                l_hat < 1.0,
                "Φ must be non-expansive on this within-cell domain (H1), measured L̂={l_hat}"
            );
            // Guard against a silently-degenerate probe reading ~0: a real contraction of
            // the efe-core pipeline is meaningfully positive.
            assert!(l_hat > 0.01, "a real Φ must show a non-trivial contraction factor, L̂={l_hat}");
        }
        other => panic!("verify_non_expansive must return an Empirical certificate, got {other:?}"),
    }
}
