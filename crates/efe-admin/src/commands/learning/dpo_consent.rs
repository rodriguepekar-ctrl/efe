//! `efe-admin learning dpo-consent {register,show}`.

use clap::Subcommand;
use chrono::{DateTime, Utc};
use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum DpoConsentCmd {
    /// Register a signed DPO consent addendum (12-month validity).
    Register {
        #[arg(long)] signer: String,
        #[arg(long)] addendum_hash: String,
        /// ISO-8601 signed_at (e.g. 2027-08-15T10:00:00Z) ; defaults to now.
        #[arg(long)] signed_at: Option<String>,
    },
    /// Show current DPO consent status.
    Show,
}

pub async fn dispatch(cmd: DpoConsentCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    match cmd {
        DpoConsentCmd::Register { signer, addendum_hash, signed_at } => {
            // Validate addendum_hash is 64 hex chars (SHA-256)
            if addendum_hash.len() != 64 || !addendum_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(AdminError::InvalidInput(
                    "addendum_hash must be 64 hex chars (SHA-256 of signed PDF)".into()
                ));
            }
            let signed = match signed_at {
                Some(s) => DateTime::parse_from_rfc3339(&s)
                    .map_err(|e| AdminError::InvalidInput(format!("bad signed_at : {}", e)))?
                    .with_timezone(&Utc),
                None => Utc::now(),
            };
            let expires = signed + chrono::Duration::days(365);
            let r = client.register_dpo_consent(
                &signer, &addendum_hash, signed, expires,
            ).await?;
            println!("✓ DPO consent registered");
            println!("  Signed at  : {}", r.signed_at);
            println!("  Expires at : {} (12 months)", r.expires_at);
            println!();
            println!("⚠ When this expires, auto-promotion will revert to false");
            println!("  unless re-signed before the deadline.");
        }
        DpoConsentCmd::Show => {
            // Re-uses get_stats which includes consent fields ; for now,
            // print a placeholder until the dedicated endpoint lands.
            let _stats = client.get_stats(None).await?;
            println!("DPO consent : check tenant_feature.m24_dpo_consent_signed_at");
            println!("              tenant_feature.m24_dpo_consent_expires_at");
            println!("(via direct DB query — dedicated endpoint pending D78)");
        }
    }
    Ok(())
}
