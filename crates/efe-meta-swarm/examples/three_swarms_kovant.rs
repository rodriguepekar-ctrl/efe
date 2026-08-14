//! The headline demo : a 3-swarm Procurement Operations Blueprint
//! collaborates via a meta-swarm.
//!
//! Three specialised swarms — supplier scoring (384), contract risk (512),
//! spend categorisation (384) — converge together in a 256-dimensional
//! meta-space.
//!
//! Run :  cargo run --release --example three_swarms_kovant -p efe-meta-swarm

use efe_meta_swarm::prelude::*;

fn main() -> Result<()> {
    let meta = MetaSwarm::builder()
        .id("kovant-procurement-v1")
        .meta_space_dim(256)
        // Three participants, three different dims, three different
        // contraction factors to simulate different convergence rates.
        .add_participant(MockParticipant::with_contraction("supplier-scoring",     384, 0.6, 11)?.boxed())
        .add_participant(MockParticipant::with_contraction("contract-risk",        512, 0.7, 22)?.boxed())
        .add_participant(MockParticipant::with_contraction("spend-categorization", 384, 0.5, 33)?.boxed())
        // Lifts : map each participant's space into the 256-dim meta-space.
        .lift_for("supplier-scoring",     LinearLift::identity(384, 256)?.boxed())
        .lift_for("contract-risk",        LinearLift::identity(512, 256)?.boxed())
        .lift_for("spend-categorization", LinearLift::identity(384, 256)?.boxed())
        // Pulls : map meta-consensus back into each participant's space.
        .pull_for("supplier-scoring",     LinearPull::identity(256, 384)?.boxed())
        .pull_for("contract-risk",        LinearPull::identity(256, 512)?.boxed())
        .pull_for("spend-categorization", LinearPull::identity(256, 384)?.boxed())
        // Meta-op : uniform 1/3-weight.  Polyhedron safety here would
        // encode the AI-Act-derived prohibited outputs ;  we use a box
        // for the demo.
        .meta_operator(UniformMetaOp::new(256).boxed())
        .meta_safety(BoxSafety::symmetric(256, 10.0).boxed())
        .alpha_default(0.5)
        .coupling_gamma(0.2)
        .tolerance(1e-4)
        .max_iters(1000)
        .trace(true)
        .build()?;

    println!("=== Procurement Meta-Swarm Demo ===");
    println!("3 swarms : supplier-scoring (384), contract-risk (512), spend-categorization (384)");
    println!("Meta-space : 256-dim, uniform aggregation, box safety.\n");

    let result = meta.iterate()?;
    println!("→ iterations      : {}", result.iterations);
    println!("→ stop reason     : {:?}", result.stop_reason);
    println!("→ final residual  : {:.4e}", result.final_residual);
    println!("→ wall time       : {:?}", result.elapsed);
    println!("→ meta consensus :");
    println!("    dim  : {}", result.meta_consensus.dim());
    println!("    ‖z*‖ : {:.4}", result.meta_consensus.norm());
    println!("    ‖z*‖∞ : {:.4}", result.meta_consensus.norm_infinity());

    println!("\n→ final participant norms :");
    for s in &result.participant_states {
        println!("    {} ‖x‖={:.4}", s.dim(), s.norm());
    }

    // ASCII convergence plot.
    if let Some(trace) = result.trace {
        let r = trace.residuals();
        ascii_residual_plot(&r, 60, 10);
    }

    Ok(())
}

fn ascii_residual_plot(residuals: &[f64], width: usize, height: usize) {
    if residuals.is_empty() { return; }
    let log: Vec<f64> = residuals.iter().map(|r| (r.max(1e-12)).log10()).collect();
    let (mn, mx) = log.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), &v|
        (mn.min(v), mx.max(v)));
    let span = (mx - mn).max(1e-9);
    let n = residuals.len();
    let stride = (n + width - 1) / width.max(1);

    println!("\nResidual trace (log scale) :");
    let mut grid = vec![vec![' '; width]; height];
    for (i, &v) in log.iter().enumerate() {
        let col = i / stride;
        if col >= width { continue; }
        let normalised = (v - mn) / span;
        let row_from_top = ((1.0 - normalised) * (height as f64 - 1.0)).round() as usize;
        let row_from_top = row_from_top.min(height - 1);
        grid[row_from_top][col] = '*';
    }
    let max_label = format!("{:.2e}", residuals.first().unwrap_or(&0.0));
    let min_label = format!("{:.2e}", residuals.last().unwrap_or(&0.0));
    println!("{:>10} ┐", max_label);
    for (i, row) in grid.iter().enumerate() {
        let bar = if i == 0 || i == height - 1 { '┤' } else { '│' };
        println!("{:>10} {} {}", "", bar, row.iter().collect::<String>());
    }
    println!("{:>10} ┘ {:─>width$}", min_label, "", width = width);
    println!("{:>10}   step 0 → step {}", "", n);
}
