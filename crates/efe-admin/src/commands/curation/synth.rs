//! `efe-admin curation synth --catalog <id> --class <id> --n <N>`

use super::SynthArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: SynthArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.generate_synth(
        &args.catalog, &args.class, args.n, &args.diversity,
        args.include_negative, &args.mode,
    ).await?;
    println!("✓ Generated {} examples for class {}", resp.examples.len(), args.class);
    println!("  Cost : {} cents | provider : {}", resp.cost_cents, resp.llm_provider);
    println!("\nProposal : {}", resp.proposal_id);
    println!("Inspect at /curation/queue/{}", resp.proposal_id);
    Ok(())
}
