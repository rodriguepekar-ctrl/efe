//! `efe-admin agent reject` — operator rejection (Art. 14).

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    approval_id: String,
    reason: String,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to reject an agent step".into(),
        ));
    }
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.reject_agent_step(&approval_id, &reason).await?;
    println!("✓ Approval rejected ({})", approval_id);
    Ok(())
}
