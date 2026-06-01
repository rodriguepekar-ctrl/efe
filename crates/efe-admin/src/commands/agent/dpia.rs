//! `efe-admin agent dpia` — DPIA workflow per tenant.

use clap::Subcommand;

use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum DpiaCmd {
    /// Sign a DPIA addendum for a tenant.
    Sign {
        #[arg(long)] tenant: String,
        #[arg(long)] addendum_hash: String,
        #[arg(long, default_value_t = 365)] valid_days: i64,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
    /// Show DPIA status for a tenant.
    Status {
        #[arg(long)] tenant: String,
    },
    /// List all tenants with DPIA status (admin only).
    List,
    /// Refresh DPIA (re-sign with same hash, push expiry forward).
    Refresh {
        #[arg(long)] tenant: String,
        #[arg(long, default_value_t = 365)] valid_days: i64,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
}

pub async fn dispatch(c: DpiaCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    match c {
        DpiaCmd::Sign { tenant, addendum_hash, valid_days, confirm } => {
            sign(tenant, addendum_hash, valid_days, confirm, ctx).await
        }
        DpiaCmd::Status { tenant } => status(tenant, ctx).await,
        DpiaCmd::List => list(ctx).await,
        DpiaCmd::Refresh { tenant, valid_days, confirm } => refresh(tenant, valid_days, confirm, ctx).await,
    }
}

async fn sign(
    tenant: String,
    addendum_hash: String,
    valid_days: i64,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to sign DPIA".into(),
        ));
    }
    if !addendum_hash.starts_with("sha256:") || addendum_hash.len() != 71 {
        return Err(AdminError::InvalidInput(
            "addendum_hash must be 'sha256:<64-hex>'".into(),
        ));
    }
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.dpia_sign(&tenant, &addendum_hash, valid_days).await?;
    println!("✓ DPIA signed for tenant '{}'", tenant);
    println!("  addendum  : {}", addendum_hash);
    println!("  valid for : {} days", valid_days);
    Ok(())
}

async fn status(tenant: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let s = crate::grpc::agent_client::AgentClient::connect(ctx).await?.dpia_status(&tenant).await?;
    println!("DPIA status for '{}'", tenant);
    println!("  signed      : {}", s.signed_at.map(|t| t.to_string()).unwrap_or("(never)".into()));
    println!("  expires     : {}", s.expires_at.map(|t| t.to_string()).unwrap_or("(n/a)".into()));
    println!("  addendum    : {}", s.addendum_hash.unwrap_or("(none)".into()));
    println!("  valid now   : {}", s.is_valid);
    Ok(())
}

async fn list(ctx: &AdminContext) -> Result<(), AdminError> {
    let rows = crate::grpc::agent_client::AgentClient::connect(ctx).await?.dpia_list().await?;
    println!("{:<24}  {:<10}  {:<24}  {:<24}", "tenant", "valid", "signed", "expires");
    for r in rows {
        println!(
            "{:<24}  {:<10}  {:<24}  {:<24}",
            r.tenant,
            if r.is_valid { "yes" } else { "no" },
            r.signed_at.map(|t| t.to_string()).unwrap_or("-".into()),
            r.expires_at.map(|t| t.to_string()).unwrap_or("-".into()),
        );
    }
    Ok(())
}

async fn refresh(
    tenant: String,
    valid_days: i64,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to refresh DPIA".into(),
        ));
    }
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.dpia_refresh(&tenant, valid_days).await?;
    println!("✓ DPIA refreshed for '{}', valid +{} days", tenant, valid_days);
    Ok(())
}
