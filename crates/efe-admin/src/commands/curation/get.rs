//! `efe-admin curation get <id>` — full proposal detail.

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(id: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let p = client.get_proposal(&id).await?;
    println!("ID         : {}", p.summary.id);
    println!("Tenant     : {}", p.summary.tenant_id);
    println!("Catalog    : {}", p.summary.catalog_id);
    println!("Kind       : {}", p.summary.kind);
    println!("Status     : {}", p.summary.status);
    println!("Proposed by: {}", p.summary.proposed_by);
    println!("Created    : {}", p.summary.created_at);
    println!(
        "Cost       : {} cents",
        p.summary.actual_cost_cents.map(|c| c.to_string()).unwrap_or_else(|| "n/a".into())
    );
    println!();
    println!("── Rationale ──");
    println!("{}", p.rationale_markdown);
    println!();
    println!("── Reviews ({}) ──", p.reviews.len());
    for r in &p.reviews {
        println!("  [{}] {} ({}) — {}",
                 r.decision, r.reviewer_oidc_sub, r.reviewer_role, r.reviewed_at);
        println!("    \"{}\"", r.rationale);
    }
    println!();
    println!("── Diff (preview) ──");
    println!("{}", serde_json::to_string_pretty(&p.diff_json)
        .unwrap_or_default().chars().take(2000).collect::<String>());
    Ok(())
}
