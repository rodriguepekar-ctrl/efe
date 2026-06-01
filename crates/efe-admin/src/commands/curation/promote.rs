//! `efe-admin curation promote <id>`

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(id: String, skip_canary: bool, ctx: &AdminContext) -> Result<(), AdminError> {
    // `skip_canary` is a guarded escape hatch — server-side RBAC (`tenant_admin`)
    // is the authoritative guard ; this CLI check is defence-in-depth.
    if skip_canary
        && std::env::var("EFE_ALLOW_SKIP_CANARY").ok().as_deref() != Some("1")
    {
        return Err(AdminError::AuthFailed(
            "skip_canary requires elevated role + EFE_ALLOW_SKIP_CANARY=1".into()
        ));
    }
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let resp = client.promote_proposal(&id, skip_canary).await?;
    println!("✓ Proposal {} promoted to candidate version {}", id, resp.candidate_version_id);
    if !skip_canary {
        println!("  Canary started at {} (24h shadow eval)", resp.canary_started_at);
        println!("  Monitor : efe-admin curation get {}", id);
    }
    Ok(())
}
