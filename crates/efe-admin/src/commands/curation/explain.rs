//! `efe-admin curation explain <sentence-id> --actual <class>`

use super::ExplainArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: ExplainArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.explain_misclassification(
        &args.catalog, &args.sentence_id, &args.actual,
        args.expected.as_deref(), &args.mode,
    ).await?;
    println!("✓ Explanation generated for sentence {}", args.sentence_id);
    println!("\n{}", resp.root_cause_markdown);
    println!("\nSuggested fixes :");
    for fix in &resp.suggested_fixes {
        println!("  • [{}] {}", fix.kind_label(), fix.description_markdown);
    }
    println!("\nProposal : {}", resp.proposal_id);
    Ok(())
}
