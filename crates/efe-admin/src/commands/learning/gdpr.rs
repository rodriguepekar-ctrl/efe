//! `efe-admin learning gdpr {erase,list-pending}`.

use clap::Subcommand;
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum GdprCmd {
    /// Submit an Art. 17 erasure request.
    Erase {
        #[arg(long)] catalog: String,
        /// Comma-separated SHA-256 hex hashes of sentences to erase.
        #[arg(long, value_delimiter = ',')] content_hashes: Vec<String>,
        #[arg(long, default_value = "STANDARD")] urgency: String,
        #[arg(long)] subject_id: Option<String>,
        #[arg(long)] reason: String,
    },
    /// List pending rebuild requests (Art. 17).
    ListPending,
}

pub async fn dispatch(cmd: GdprCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        GdprCmd::Erase { catalog, content_hashes, urgency, subject_id, reason } => {
            if content_hashes.is_empty() {
                return Err(AdminError::InvalidInput("content_hashes must not be empty".into()));
            }
            if reason.trim().len() < 10 {
                return Err(AdminError::InvalidInput("reason must be ≥ 10 chars".into()));
            }
            let r = client.submit_gdpr_erasure(
                &catalog, &urgency, &content_hashes,
                subject_id.as_deref(), &reason,
            ).await?;
            println!("✓ GDPR Art. 17 erasure submitted");
            println!("  Rebuild request : {}", r.rebuild_request_id);
            println!("  Affected count  : {}", r.affected_count);
            println!("  SLA deadline    : {}", r.sla_deadline);
        }
        GdprCmd::ListPending => {
            let items = client.list_gdpr_pending().await?;
            println!("{:<14} {:<20} {:<13} {:<10} {:<25} {}",
                     "ID", "CATALOG", "URGENCY", "AFFECTED", "SLA DEADLINE", "STATUS");
            println!("{}", "─".repeat(95));
            for it in &items {
                println!("{:<14} {:<20} {:<13} {:<10} {:<25} {}",
                    &it.id[..14.min(it.id.len())], it.catalog_id, it.urgency,
                    it.affected_count, it.sla_deadline.format("%Y-%m-%d %H:%M"),
                    it.status);
            }
            println!("\n{} pending requests.", items.len());
        }
    }
    Ok(())
}
