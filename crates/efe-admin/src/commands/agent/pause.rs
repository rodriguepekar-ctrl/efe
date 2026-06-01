//! `efe-admin agent pause` — pause an active run.

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    run_id: String,
    reason: Option<String>,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.pause_agent_run(&run_id, reason.as_deref()).await?;
    println!("✓ Run paused : {}", run_id);
    Ok(())
}
