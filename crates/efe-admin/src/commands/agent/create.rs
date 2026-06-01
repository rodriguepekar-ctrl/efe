//! `efe-admin agent create` — instantiate an AgentRun from a template.

use crate::context::AdminContext;
use crate::error::AdminError;
use std::path::PathBuf;

pub async fn run(
    tenant: String,
    template: String,
    prompt_file: Option<String>,
    timeout_seconds: Option<i32>,
    max_cost_usd: Option<f64>,
    tools: Option<Vec<String>>,
    ctx: &AdminContext,
) -> Result<(), AdminError> {
    let prompt_json = match prompt_file {
        Some(path) => {
            let raw = std::fs::read_to_string(PathBuf::from(&path))
                .map_err(|e| AdminError::InvalidInput(format!("read prompt file {path} : {e}")))?;
            serde_json::from_str::<serde_json::Value>(&raw)
                .map_err(|e| AdminError::InvalidInput(format!("parse prompt JSON : {e}")))?
        }
        None => serde_json::json!({"messages": [{"role": "user", "content": "<set EFE_INITIAL_USER_PROMPT or pass --prompt-file>"}]}),
    };

    let req = crate::grpc::agent_client::AgentClient::connect(ctx).await?.create_agent_run(crate::grpc::agent_client::CreateAgentRunArgs {
        tenant_id: tenant.clone(),
        template_slug: template,
        initial_prompt: prompt_json,
        tool_whitelist: tools,
        timeout_seconds,
        max_cost_usd,
    }).await?;

    println!("✓ Agent run created");
    println!("  id        : {}", req.id);
    println!("  tenant    : {}", req.tenant_id);
    println!("  status    : {}", req.status);
    println!("  profile   : {}", req.profile);
    println!("  max_cost  : ${:.2}", req.max_cost_cents as f64 / 100.0);
    println!("  follow    : `efe-admin agent tail {}`", req.id);
    Ok(())
}
