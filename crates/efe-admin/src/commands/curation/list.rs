//! `efe-admin curation list` — list pending proposals.

use super::{ListArgs, OutputFormat};
use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(args: ListArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let proposals = client.list_pending(args.catalog.as_deref(), args.limit).await?;

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&proposals)?);
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(&proposals)?);
        }
        OutputFormat::Table => {
            println!("{:<14} {:<6} {:<22} {:<6} {:<8} {:<10} {}",
                     "ID", "KIND", "CATALOG", "AGE", "STATUS", "REVIEWS", "PROPOSER");
            println!("{}", "─".repeat(95));
            for p in &proposals {
                let age = format_age(&p.created_at);
                println!("{:<14} {:<6} {:<22} {:<6} {:<8} {:<10} {}",
                         &p.id[..14.min(p.id.len())], p.kind, p.catalog_id, age,
                         p.status, p.review_count, p.proposed_by);
            }
            println!("\n{} proposals matching filter.", proposals.len());
        }
    }
    Ok(())
}

fn format_age(ts: &chrono::DateTime<chrono::Utc>) -> String {
    let delta = chrono::Utc::now().signed_duration_since(*ts);
    if delta.num_minutes() < 60 { format!("{}m", delta.num_minutes()) }
    else if delta.num_hours() < 24 { format!("{}h", delta.num_hours()) }
    else { format!("{}d", delta.num_days()) }
}
