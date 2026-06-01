//! `efe-admin curation propose-split --catalog <id> --class <id>`

use super::ProposeSplitArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: ProposeSplitArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    if args.dry_run {
        println!("[dry-run] would propose split for {}/{} (max_splits={})",
                 args.catalog, args.class, args.max_splits);
        return Ok(());
    }
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.propose_split(
        &args.catalog, &args.class,
        args.max_splits, args.sample_size, &args.mode,
    ).await?;
    println!("✓ Proposal created : {}", resp.proposal_id);
    println!("  Provider     : {}", resp.llm_provider);
    println!("  Cost         : {} cents", resp.cost_cents);
    println!("  Subclasses   : {}", resp.proposed_classes.len());
    for c in &resp.proposed_classes {
        println!("    - {} ({} anchors)", c.proposed_class_id, c.anchor_candidates.len());
    }
    println!("\nReview at /curation/queue/{}", resp.proposal_id);
    Ok(())
}
