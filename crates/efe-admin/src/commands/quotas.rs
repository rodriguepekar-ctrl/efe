//! `efe-admin quotas <get|set>`.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{QuotasArgs, QuotasCommand, QuotasGetArgs, QuotasSetArgs};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::prompts::confirm;
use crate::validation;

// `quotas get` reuses M16 `efe_get_admin_quotas` (read-only).
// `quotas set` is a fresh D8 tool (write + confirm).
const TOOL_GET: &str = "efe_get_admin_quotas";
const TOOL_SET: &str = "efe_set_quotas";

pub async fn dispatch(args: QuotasArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        QuotasCommand::Get(a) => get(a, ctx).await,
        QuotasCommand::Set(a) => set(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Quotas {
    tenant_id: String,
    max_rps: u32,
    max_tokens_per_day: u64,
    monthly_cap_usd: Option<f64>,
    used_today_tokens: Option<u64>,
    used_today_rps_p99: Option<u32>,
}

async fn get(a: QuotasGetArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    let resp = ctx.mcp().call_tool(TOOL_GET, json!({"tenant_id": a.tenant})).await?;
    let parsed: Quotas = serde_json::from_value(resp)?;
    ctx.renderer.print(&parsed)
}

async fn set(a: QuotasSetArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    if a.max_rps.is_none() && a.max_tokens_per_day.is_none() {
        return Err(AdminError::invalid("supply at least --max-rps or --max-tokens-per-day"));
    }
    if !ctx.confirm_bypass()
        && !confirm(
            &format!("Set quotas for tenant '{}' ?", a.tenant),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let mut payload = json!({"tenant_id": a.tenant, "confirm": true});
    if let Some(v) = a.max_rps {
        payload["max_rps"] = json!(v);
    }
    if let Some(v) = a.max_tokens_per_day {
        payload["max_tokens_per_day"] = json!(v);
    }
    let resp = ctx.mcp().call_tool(TOOL_SET, payload).await?;
    ctx.renderer.print(&resp)
}
