//! `efe-admin health` — overall cluster health snapshot.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::HealthArgs;
use crate::context::AdminContext;
use crate::error::AdminError;

// `health` reuses the M17 audit-chain health checker as a cheap
// liveness probe across the stack. A future GLOSS-B will add a
// proper multi-component cluster health aggregator.
const TOOL: &str = "efe_audit_health_check";

#[derive(Debug, Serialize, Deserialize)]
struct ClusterHealth {
    status: String,
    efe_server: SubcheckStatus,
    mcp_server: SubcheckStatus,
    postgres: SubcheckStatus,
    redis: SubcheckStatus,
    audit_chain: SubcheckStatus,
    metrics: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct SubcheckStatus {
    status: String,
    details: Option<String>,
    last_check: String,
}

pub async fn run(_a: HealthArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let resp = ctx.mcp().call_tool(TOOL, json!({})).await?;
    let parsed: ClusterHealth = serde_json::from_value(resp)?;
    ctx.renderer.print(&parsed)
}
