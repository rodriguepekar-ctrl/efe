// SPDX-License-Identifier: UNLICENSED
//! **L'invariant de non-interférence (I-1)** — activer un [`StepObserver`] ne change **RIEN** au
//! numérique. Testé sans aucune feature : l'observateur est crypto-free et toujours compilé.

use efe_meta_swarm::prelude::*;
use efe_meta_swarm::{ObservedStep, StepObserver};

/// Un observateur trivial : il compte les pas, retient le dernier résidu et le dernier `safety_delta`.
/// Aucun effet de bord sur l'itération.
struct CountingObserver {
    count: usize,
    last_residual: f64,
    last_safety_delta: f64,
}
impl StepObserver for CountingObserver {
    fn on_step(&mut self, o: &ObservedStep) {
        self.count += 1;
        self.last_residual = o.residual;
        self.last_safety_delta = o.safety_delta;
    }
}

/// Un petit méta-essaim déterministe (seeds figés) — reconstruit à neuf à chaque appel.
fn build() -> MetaSwarm {
    MetaSwarm::builder()
        .id("test-meta")
        .meta_space_dim(4)
        .add_participant(MockParticipant::with_contraction("a", 8, 0.6, 1).unwrap().boxed())
        .add_participant(MockParticipant::with_contraction("b", 8, 0.5, 2).unwrap().boxed())
        .lift_for("a", LinearLift::identity(8, 4).unwrap().boxed())
        .lift_for("b", LinearLift::identity(8, 4).unwrap().boxed())
        .pull_for("a", LinearPull::identity(4, 8).unwrap().boxed())
        .pull_for("b", LinearPull::identity(4, 8).unwrap().boxed())
        .meta_operator(UniformMetaOp::new(4).boxed())
        .meta_safety(BoxSafety::symmetric(4, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-6)
        .max_iters(200)
        .build()
        .unwrap()
}

#[test]
fn observer_does_not_perturb_numerics() {
    // Sans observateur.
    let plain = build().iterate().unwrap();
    // Avec observateur (instance fraîche → mêmes seeds).
    let mut obs = CountingObserver { count: 0, last_residual: 0.0, last_safety_delta: -1.0 };
    let observed = build().iterate_observed(&mut obs).unwrap();

    assert_eq!(plain.iterations, observed.iterations, "mêmes itérations");
    assert_eq!(plain.final_residual.to_bits(), observed.final_residual.to_bits(), "même résidu final (bit-à-bit)");
    assert_eq!(plain.meta_consensus.as_slice(), observed.meta_consensus.as_slice(), "même méta-consensus");
    assert_eq!(format!("{:?}", plain.stop_reason), format!("{:?}", observed.stop_reason), "même raison d'arrêt");

    // L'observateur a bien vu un pas par itération, le dernier résidu concorde.
    assert_eq!(obs.count, observed.iterations, "un on_step par pas");
    assert_eq!(obs.last_residual.to_bits(), observed.final_residual.to_bits(), "dernier résidu observé");
    // Le safety_delta observé est un ‖·‖ valide (fini, ≥ 0).
    assert!(obs.last_safety_delta >= 0.0 && obs.last_safety_delta.is_finite(), "safety_delta valide");
}
