//! `efe-admin agent resume` — resume a paused run.

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(run_id: String, ctx: &AdminContext) -> Result<(), AdminError> {
    crate::grpc::agent_client::AgentClient::connect(ctx).await?.resume_agent_run(&run_id).await?;
    println!("✓ Run resumed : {}", run_id);
    Ok(())
}
