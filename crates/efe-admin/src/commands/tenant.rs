//! `efe-admin tenant <subcommand>` — tenant lifecycle.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{TenantArgs, TenantCommand, TenantCreateArgs, TenantDeleteArgs, TenantGetArgs,
                  TenantListArgs, TenantSuspendArgs, TenantUpdateArgs};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::prompts::{confirm, confirm_typed};
use crate::validation;

const TOOL_CREATE: &str = "efe_create_tenant";
const TOOL_LIST: &str = "efe_list_tenants";
const TOOL_GET: &str = "efe_get_tenant";
const TOOL_UPDATE: &str = "efe_update_tenant";
const TOOL_SUSPEND: &str = "efe_suspend_tenant";
const TOOL_DELETE: &str = "efe_delete_tenant";

pub async fn dispatch(args: TenantArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        TenantCommand::Create(a) => create(a, ctx).await,
        TenantCommand::List(a) => list(a, ctx).await,
        TenantCommand::Get(a) => get(a, ctx).await,
        TenantCommand::Update(a) => update(a, ctx).await,
        TenantCommand::Suspend(a) => suspend(a, ctx).await,
        TenantCommand::Delete(a) => delete(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TenantCreateOutput {
    tenant_id: String,
    plan: String,
    max_rps: u32,
    data_residency: String,
    created_at: String,
    initial_admin_token: String,
}

async fn create(a: TenantCreateArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.name)?;
    if let Some(email) = &a.contact_email {
        validation::email(email)?;
    }

    if !ctx.confirm_bypass() {
        let prompt = format!(
            "Create tenant '{}' (plan={}, max_rps={}, residency={}) ?",
            a.name, a.plan, a.max_rps, a.data_residency
        );
        if !confirm(&prompt, ctx.config.confirm_mode)? {
            return Err(AdminError::UserAbort);
        }
    }

    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_CREATE,
            json!({
                "name": a.name,
                "plan": a.plan,
                "max_rps": a.max_rps,
                "max_tokens_per_day": a.max_tokens_per_day,
                "data_residency": a.data_residency,
                "contact_email": a.contact_email,
                "confirm": true,
            }),
        )
        .await?;

    let parsed: TenantCreateOutput = serde_json::from_value(resp)?;
    eprintln!(
        "✅ tenant '{}' created — {} plan, {} RPS, residency={}",
        parsed.tenant_id, parsed.plan, parsed.max_rps, parsed.data_residency
    );
    eprintln!("⚠️  initial admin token (shown ONCE, save now):");
    eprintln!("    {}", parsed.initial_admin_token);
    ctx.renderer.print(&parsed)
}

#[derive(Debug, Serialize, Deserialize)]
struct TenantRow {
    name: String,
    plan: String,
    status: String,
    max_rps: u32,
    data_residency: String,
    created_at: String,
}

async fn list(a: TenantListArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    let mut payload = json!({});
    if let Some(s) = &a.status {
        payload["status"] = json!(s);
    }
    let resp = ctx.mcp().call_tool(TOOL_LIST, payload).await?;
    let rows: Vec<TenantRow> = serde_json::from_value(
        resp.get("tenants").cloned().unwrap_or_else(|| serde_json::Value::Array(vec![])),
    )?;
    ctx.renderer.print(&rows)
}

#[derive(Debug, Serialize, Deserialize)]
struct TenantDetail {
    name: String,
    plan: String,
    status: String,
    max_rps: u32,
    max_tokens_per_day: u64,
    data_residency: String,
    contact_email: Option<String>,
    created_at: String,
    suspended_at: Option<String>,
    suspended_reason: Option<String>,
    api_keys_active: u32,
    catalogs: u32,
    last_classify_at: Option<String>,
}

async fn get(a: TenantGetArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.name)?;
    let resp = ctx.mcp().call_tool(TOOL_GET, json!({"tenant_id": a.name})).await?;
    let detail: TenantDetail = serde_json::from_value(resp)?;
    ctx.renderer.print(&detail)
}

async fn update(a: TenantUpdateArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.name)?;
    if a.max_rps.is_none()
        && a.max_tokens_per_day.is_none()
        && a.data_residency.is_none()
        && a.contact_email.is_none()
    {
        return Err(AdminError::invalid("nothing to update — supply at least one field"));
    }

    if !ctx.confirm_bypass() {
        let prompt = format!("Update tenant '{}' ?", a.name);
        if !confirm(&prompt, ctx.config.confirm_mode)? {
            return Err(AdminError::UserAbort);
        }
    }

    let mut payload = json!({"tenant_id": a.name, "confirm": true});
    if let Some(v) = a.max_rps {
        payload["max_rps"] = json!(v);
    }
    if let Some(v) = a.max_tokens_per_day {
        payload["max_tokens_per_day"] = json!(v);
    }
    if let Some(v) = a.data_residency {
        payload["data_residency"] = json!(v);
    }
    if let Some(v) = a.contact_email {
        validation::email(&v)?;
        payload["contact_email"] = json!(v);
    }

    let resp = ctx.mcp().call_tool(TOOL_UPDATE, payload).await?;
    ctx.renderer.print(&resp)
}

async fn suspend(a: TenantSuspendArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.name)?;
    if a.reason.trim().len() < 10 {
        return Err(AdminError::invalid("suspend reason must be ≥ 10 chars"));
    }
    if !ctx.confirm_bypass()
        && !confirm(
            &format!("Suspend tenant '{}' ? This sets max_rps=0 immediately.", a.name),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_SUSPEND,
            json!({"tenant_id": a.name, "reason": a.reason, "confirm": true}),
        )
        .await?;
    eprintln!("⚠️  tenant '{}' suspended", a.name);
    ctx.renderer.print(&resp)
}

async fn delete(a: TenantDeleteArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.name)?;
    if !ctx.confirm_bypass()
        && !confirm_typed(&a.name, ctx.config.confirm_mode)?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_DELETE,
            json!({
                "tenant_id": a.name,
                "purge_audit_after_days": a.purge_audit_after_days,
                "confirm": true,
            }),
        )
        .await?;
    eprintln!("🗑️  tenant '{}' deleted", a.name);
    ctx.renderer.print(&resp)
}
