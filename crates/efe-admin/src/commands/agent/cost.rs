//! `efe-admin agent cost` — cost reporting (per tenant + per run).

use clap::Subcommand;

use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum CostCmd {
    /// Daily total per tenant.
    Daily {
        #[arg(long)] tenant: String,
        #[arg(long)] days: Option<i32>,
    },
    /// Single run cost breakdown by step.
    Run { run_id: String },
}

pub async fn dispatch(c: CostCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    match c {
        CostCmd::Daily { tenant, days } => daily(tenant, days.unwrap_or(7), ctx).await,
        CostCmd::Run { run_id } => run(run_id, ctx).await,
    }
}

async fn daily(tenant: String, days: i32, ctx: &AdminContext) -> Result<(), AdminError> {
    let rows = crate::grpc::agent_client::AgentClient::connect(ctx).await?.cost_daily(&tenant, days).await?;
    println!("{:<12}  {:>10}  {:>6}", "day", "cost(c)", "runs");
    for r in rows {
        println!("{:<12}  {:>10}  {:>6}", r.day, r.cents, r.runs);
    }
    Ok(())
}

async fn run(run_id: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let r = crate::grpc::agent_client::AgentClient::connect(ctx).await?.cost_per_run(&run_id).await?;
    println!("Run {} cost breakdown :", run_id);
    println!("  total      : ${:.2}", r.total_cents as f64 / 100.0);
    println!("  llm calls  : {} (${:.2})", r.llm_calls, r.llm_cents as f64 / 100.0);
    println!("  tool calls : {}", r.tool_calls);
    Ok(())
}
