//! Multimodal scenario : three swarms of *different* latent dimensions
//! collaborate via a meta-swarm.
//!
//! Run :  cargo run --example multimodal_three_swarms -p efe-meta-swarm

use efe_meta_swarm::prelude::*;

fn main() -> Result<()> {
    let meta = MetaSwarm::builder()
        .id("multimodal-three-swarms")
        .meta_space_dim(128)
        .add_participant(MockParticipant::with_contraction("text",   384,  0.5, 1)?.boxed())
        .add_participant(MockParticipant::with_contraction("vision", 1024, 0.6, 2)?.boxed())
        .add_participant(MockParticipant::with_contraction("audio",  768,  0.55, 3)?.boxed())
        .lift_for("text",   LinearLift::identity(384, 128)?.boxed())
        .lift_for("vision", LinearLift::identity(1024, 128)?.boxed())
        .lift_for("audio",  LinearLift::identity(768, 128)?.boxed())
        .pull_for("text",   LinearPull::identity(128, 384)?.boxed())
        .pull_for("vision", LinearPull::identity(128, 1024)?.boxed())
        .pull_for("audio",  LinearPull::identity(128, 768)?.boxed())
        .meta_operator(UniformMetaOp::new(128).boxed())
        .meta_safety(BoxSafety::symmetric(128, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(800)
        .build()?;

    println!("=== Multimodal meta-swarm ===");
    println!("text=384, vision=1024, audio=768, meta=128\n");

    let result = meta.iterate()?;
    println!("→ iterations     : {}", result.iterations);
    println!("→ final residual : {:.4e}", result.final_residual);
    println!("→ stop reason    : {:?}", result.stop_reason);
    println!("→ wall time      : {:?}", result.elapsed);
    Ok(())
}
