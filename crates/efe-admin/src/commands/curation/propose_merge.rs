//! `efe-admin curation propose-merge --catalog <id> --classes a,b[,c]`

use super::ProposeMergeArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: ProposeMergeArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    if !(2..=3).contains(&args.classes.len()) {
        return Err(AdminError::InvalidInput("--classes must list 2 or 3 IDs".into()));
    }
    if args.dry_run {
        println!("[dry-run] would propose merge of {:?} in catalog {}", args.classes, args.catalog);
        return Ok(());
    }
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.propose_merge(&args.catalog, &args.classes, &args.mode).await?;
    println!("✓ Merge proposal : {}", resp.proposal_id);
    println!("  Merging       : {}", args.classes.join(", "));
    println!("  Cost          : {} cents", resp.cost_cents);
    println!("\nReview at /curation/queue/{}", resp.proposal_id);
    Ok(())
}
