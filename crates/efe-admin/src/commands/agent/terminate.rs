//! `efe-admin agent terminate` — force-stop a run (microVM SIGKILL).

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    run_id: String,
    reason: String,
    confirm: bool,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to terminate".into(),
        ));
    }
    if reason.trim().len() < 10 {
        return Err(AdminError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.terminate_agent_run(&run_id, &reason).await?;
    println!("✓ Run terminated : {}", run_id);
    Ok(())
}
