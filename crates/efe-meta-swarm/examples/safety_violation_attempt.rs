//! Demonstrates Theorem 5.1 of A5 *visually* : initialise the swarms
//! deliberately outside their safety set, then watch the projection
//! force them back in within one iteration.
//!
//! Run :  cargo run --example safety_violation_attempt -p efe-meta-swarm

use efe_meta_swarm::prelude::*;

fn main() -> Result<()> {
    let a = MockParticipant::with_contraction("a", 8, 0.5, 1)?
        .with_safety(BoxSafety::symmetric(8, 1.0).boxed());
    let b = MockParticipant::with_contraction("b", 8, 0.5, 2)?
        .with_safety(BoxSafety::symmetric(8, 1.0).boxed());

    // Push both participants outside their box (values ≈ ±5).
    a.set_state(Tensor::pseudo_random(8, 10).scale(5.0))?;
    b.set_state(Tensor::pseudo_random(8, 11).scale(5.0))?;

    let meta = MetaSwarm::builder()
        .id("safety_violation_attempt")
        .meta_space_dim(8)
        .add_participant(Box::new(a))
        .add_participant(Box::new(b))
        .lift_for("a", LinearLift::identity(8, 8)?.boxed())
        .lift_for("b", LinearLift::identity(8, 8)?.boxed())
        .pull_for("a", LinearPull::identity(8, 8)?.boxed())
        .pull_for("b", LinearPull::identity(8, 8)?.boxed())
        .meta_operator(UniformMetaOp::new(8).boxed())
        .meta_safety(BoxSafety::symmetric(8, 1.0).boxed())
        .alpha_default(0.4)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(100)
        .trace(true)
        .build()?;

    let result = meta.iterate()?;
    let safety = BoxSafety::symmetric(8, 1.0);

    println!("=== Safety violation attempt ===\n");
    if let Some(trace) = result.trace {
        for (i, step) in trace.steps().iter().enumerate().take(5) {
            for (j, s) in step.participant_states.iter().enumerate() {
                let inside = safety.contains(s, 1e-6);
                println!("  step {i} · participant {j} · ‖x‖∞={:.4} · inside={inside}", s.norm_infinity());
            }
        }
        println!("  …");
    }
    println!("\n→ final residual : {:.4e}", result.final_residual);
    println!("→ all final states inside safety set : {}",
        result.participant_states.iter().all(|s| safety.contains(s, 1e-6)));
    Ok(())
}
