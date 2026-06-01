//! `efe-admin agent tail` — stream agent steps live (server-side polling).

use crate::context::AdminContext;
use crate::error::AdminError;
use std::time::Duration;

pub async fn run(run_id: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let mut last_step = -1i32;
    println!("Tailing run {} (Ctrl-C to stop)", run_id);
    loop {
        let d = crate::grpc::agent_client::AgentClient::connect(ctx).await?.get_agent_run(&run_id).await?;
        for s in d.recent_steps {
            if s.step_index > last_step {
                println!("  [{:>3}] {:<14} cost={}c", s.step_index, s.kind, s.cost_cents);
                last_step = s.step_index;
            }
        }
        if matches!(d.run.status.as_str(),
            "COMPLETED" | "FAILED" | "TERMINATED" | "TIMED_OUT" | "COST_CAPPED")
        {
            println!("→ run reached terminal status : {}", d.run.status);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
