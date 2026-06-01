//! `efe-admin keys <subcommand>` — API key lifecycle.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{KeyCreateArgs, KeyListArgs, KeyRevokeArgs, KeyRotateArgs, KeysArgs, KeysCommand};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::prompts::confirm;
use crate::validation;

const TOOL_CREATE: &str = "efe_create_api_key";
const TOOL_ROTATE: &str = "efe_rotate_api_key";
const TOOL_REVOKE: &str = "efe_revoke_api_key";
const TOOL_LIST: &str = "efe_list_api_keys";

pub async fn dispatch(args: KeysArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        KeysCommand::Create(a) => create(a, ctx).await,
        KeysCommand::Rotate(a) => rotate(a, ctx).await,
        KeysCommand::Revoke(a) => revoke(a, ctx).await,
        KeysCommand::List(a) => list(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyCreateOutput {
    key_id: String,
    plaintext: String,
    scope: String,
    expires_at: Option<String>,
}

async fn create(a: KeyCreateArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    let expires_in = validation::duration_or_never(&a.expires_in)?;
    let expires_at = expires_in.map(|d| Utc::now() + d);

    if !ctx.confirm_bypass()
        && !confirm(
            &format!("Create {} key for tenant '{}' (expires_in={}) ?", a.scope, a.tenant, a.expires_in),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_CREATE,
            json!({
                "tenant_id": a.tenant,
                "scope": a.scope,
                "expires_at": expires_at.map(|t| t.to_rfc3339()),
                "confirm": true,
            }),
        )
        .await?;

    let parsed: KeyCreateOutput = serde_json::from_value(resp)?;
    eprintln!("⚠️  API key (shown ONCE — save now):");
    eprintln!("    {}", parsed.plaintext);
    eprintln!("    key_id: {} (scope: {})", parsed.key_id, parsed.scope);
    if let Some(exp) = &parsed.expires_at {
        eprintln!("    expires_at: {exp}");
    }
    ctx.renderer.print(&parsed)
}

async fn rotate(a: KeyRotateArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    if !ctx.confirm_bypass()
        && !confirm(
            &format!(
                "Rotate key {} for tenant '{}' ? Old key will be invalidated after 24h grace.",
                a.key_id, a.tenant
            ),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_ROTATE,
            json!({
                "tenant_id": a.tenant,
                "key_id": a.key_id,
                "confirm": true,
            }),
        )
        .await?;
    ctx.renderer.print(&resp)
}

async fn revoke(a: KeyRevokeArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    if a.reason.trim().len() < 10 {
        return Err(AdminError::invalid("revoke reason must be ≥ 10 chars"));
    }
    if !ctx.confirm_bypass()
        && !confirm(
            &format!("Revoke key {} for tenant '{}' ? Effective immediately.", a.key_id, a.tenant),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_REVOKE,
            json!({
                "tenant_id": a.tenant,
                "key_id": a.key_id,
                "reason": a.reason,
                "confirm": true,
            }),
        )
        .await?;
    ctx.renderer.print(&resp)
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyRow {
    key_id: String,
    suffix: String,
    scope: String,
    created_at: String,
    expires_at: Option<String>,
    revoked_at: Option<String>,
    last_used_at: Option<String>,
    use_count: u64,
}

async fn list(a: KeyListArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_LIST,
            json!({"tenant_id": a.tenant, "include_revoked": a.all}),
        )
        .await?;
    let rows: Vec<KeyRow> = serde_json::from_value(
        resp.get("keys").cloned().unwrap_or_else(|| serde_json::Value::Array(vec![])),
    )?;
    ctx.renderer.print(&rows)
}
