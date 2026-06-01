//! `efe-admin agent approve` — operator approval (Art. 14).

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
            "must pass --confirm to approve an agent step".into(),
        ));
    }
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    let token = crate::grpc::agent_client::AgentClient::connect(ctx).await?.approve_agent_step(&approval_id, &reason).await?;
    println!("✓ Approval granted");
    println!("  approval_id   : {}", approval_id);
    println!("  expires_at    : {}", token.expires_at);
    println!("  (token forwarded to agent-core via vsock-relay ; not printed)");
    Ok(())
}
