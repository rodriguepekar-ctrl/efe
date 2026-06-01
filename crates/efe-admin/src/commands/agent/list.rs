//! `efe-admin agent list` — list active runs (or include completed with --all).

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(
    tenant: String,
    all: bool,
    limit: i32,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    let runs = crate::grpc::agent_client::AgentClient::connect(ctx)
        .await?
        .list_agent_runs(&tenant, all, limit)
        .await?;
    if runs.is_empty() {
        println!("(no agent runs for tenant '{}')", tenant);
        return Ok(());
    }
    println!(
        "{:<36}  {:<12}  {:<14}  {:<8}  {:<10}  {:<8}",
        "id", "status", "template", "step", "cost(c)", "age"
    );
    for r in runs {
        let age = chrono::Utc::now()
            .signed_duration_since(r.created_at)
            .num_minutes();
        println!(
            "{:<36}  {:<12}  {:<14}  {:>3}/{:<3}  {:>10}  {:>4}m",
            r.id,
            r.status,
            r.template_slug,
            r.current_step,
            r.max_steps,
            r.current_cost_cents,
            age
        );
    }
    Ok(())
}
