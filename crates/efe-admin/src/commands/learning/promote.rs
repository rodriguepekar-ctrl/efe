//! `efe-admin learning promote <run-id>`.

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(run_id: String, skip_canary: bool, ctx: &AdminContext) -> Result<(), AdminError> {
    if skip_canary && std::env::var("EFE_ALLOW_SKIP_CANARY").ok().as_deref() != Some("1") {
        return Err(AdminError::AuthFailed(
            "skip_canary requires EFE_ALLOW_SKIP_CANARY=1 + tenant_admin role".into()
        ));
    }
    let client = crate::grpc::active_learning_client::ActiveLearningClient::connect(ctx).await?;
    let resp = client.promote(&run_id, skip_canary).await?;
    println!("✓ Retune {} promoted → new ACTIVE version : {}",
             run_id, resp.new_active_taxonomy_version_id);
    if !skip_canary {
        println!("  Canary started at {} (24h shadow eval)", resp.canary_started_at);
    }
    Ok(())
}
