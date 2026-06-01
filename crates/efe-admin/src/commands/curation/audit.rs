//! `efe-admin curation audit --bundle <id> --gold <id>`

use super::AuditArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: AuditArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.audit_bundle_vs_gold(
        &args.catalog, &args.bundle, &args.gold,
        args.max_weak_spots, args.with_llm, &args.mode,
    ).await?;
    println!("Audit scorecard : {}", resp.scorecard_id);
    println!("  Bundle      : {}", args.bundle);
    println!("  Gold        : {}", args.gold);
    println!("  Accuracy    : {:.4}", resp.accuracy);
    println!("  Macro F1    : {:.4}", resp.macro_f1);
    println!("  Weighted F1 : {:.4}", resp.weighted_f1);
    println!("\nWeak spots : {}", resp.weak_spots.len());
    for w in &resp.weak_spots {
        println!("  • [{:.2}] {} : {}", w.severity, w.class_id, w.description);
    }
    if args.with_llm {
        println!("\nLLM analysis :\n{}", resp.llm_explanation.unwrap_or_default());
        println!("\nCost : {} cents", resp.cost_cents);
    }
    Ok(())
}
