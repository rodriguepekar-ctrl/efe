//! `efe-admin agent get` — single-run detail.

use crate::context::AdminContext;
use crate::error::AdminError;

pub async fn run(run_id: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let detail = crate::grpc::agent_client::AgentClient::connect(ctx).await?.get_agent_run(&run_id).await?;
    let r = detail.run;
    println!("Agent run {}", r.id);
    println!("  status    : {}", r.status);
    println!("  tenant    : {}", r.tenant_id);
    println!("  template  : {}", r.template_slug);
    println!("  profile   : {}", r.profile);
    println!("  provider  : {}/{}", r.provider, r.model);
    println!("  step      : {}/{}", r.current_step, r.max_steps);
    println!("  cost      : ${:.2} / ${:.2}",
        r.current_cost_cents as f64 / 100.0,
        r.max_cost_cents as f64 / 100.0,
    );
    println!("  created   : {}", r.created_at);
    if let Some(t) = r.started_at { println!("  started   : {}", t); }
    if let Some(t) = r.completed_at { println!("  completed : {}", t); }
    if let Some(reason) = &r.failure_reason { println!("  failure   : {}", reason); }

    if !detail.recent_steps.is_empty() {
        println!("\nRecent steps :");
        for s in detail.recent_steps {
            println!("  [{}] {} cost={}c", s.step_index, s.kind, s.cost_cents);
        }
    }
    if !detail.pending_approvals.is_empty() {
        println!("\nPending approvals :");
        for a in detail.pending_approvals {
            println!("  {} step={} tool={} expires={}",
                a.id, a.step_index, a.tool_name, a.expires_at);
        }
    }
    Ok(())
}
