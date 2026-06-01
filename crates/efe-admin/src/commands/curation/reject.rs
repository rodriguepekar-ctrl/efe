//! `efe-admin curation reject <id> --reason <...>`

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(id: String, reason: String, ctx: &AdminContext) -> Result<(), AdminError> {
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput(
            "rationale must be ≥ 10 chars".into()
        ));
    }
    let client = crate::grpc::curation_client::CurationClient::connect(ctx).await?;
    let new_status = client.reject_proposal(&id, &reason).await?;
    println!("✓ Proposal {} rejected → status = {}", id, new_status);
    Ok(())
}
