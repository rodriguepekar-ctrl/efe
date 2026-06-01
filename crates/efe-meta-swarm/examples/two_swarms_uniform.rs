//! Simplest possible example : two identical-dim swarms collaborate via
//! a uniform meta-operator.
//!
//! Run :  cargo run --example two_swarms_uniform -p efe-meta-swarm

use efe_meta_swarm::prelude::*;

fn main() -> Result<()> {
    let meta = MetaSwarm::builder()
        .id("two_swarms_uniform")
        .meta_space_dim(8)
        .add_participant(MockParticipant::with_contraction("a", 16, 0.6, 1)?.boxed())
        .add_participant(MockParticipant::with_contraction("b", 16, 0.6, 2)?.boxed())
        .lift_for("a", LinearLift::identity(16, 8)?.boxed())
        .lift_for("b", LinearLift::identity(16, 8)?.boxed())
        .pull_for("a", LinearPull::identity(8, 16)?.boxed())
        .pull_for("b", LinearPull::identity(8, 16)?.boxed())
        .meta_operator(UniformMetaOp::new(8).boxed())
        .meta_safety(BoxSafety::symmetric(8, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(500)
        .trace(true)
        .build()?;

    println!("=== two_swarms_uniform ===");
    println!("Configuration : {meta:?}\n");

    let result = meta.iterate()?;
    println!("→ iterations   : {}", result.iterations);
    println!("→ stop reason  : {:?}", result.stop_reason);
    println!("→ final residual : {:.2e}", result.final_residual);
    println!("→ elapsed      : {:?}", result.elapsed);
    println!("→ meta consensus norm : {:.4}", result.meta_consensus.norm());
    for (i, s) in result.participant_states.iter().enumerate() {
        println!("→ participant {i} final norm : {:.4}", s.norm());
    }

    if let Some(trace) = result.trace {
        let r = trace.residuals();
        let first = r.first().copied().unwrap_or(0.0);
        let last = r.last().copied().unwrap_or(0.0);
        println!("→ residual trajectory : {first:.4e} → {last:.4e} over {} steps", r.len());
    }

    Ok(())
}
