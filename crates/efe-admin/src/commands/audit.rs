//! `efe-admin audit <query|export>` — read audit log + export to JSONL.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::cli::{AuditArgs, AuditCommand, AuditExportArgs, AuditQueryArgs};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::validation;

const TOOL_QUERY: &str = "efe_get_audit_log";
const TOOL_EXPORT: &str = "efe_export_audit_log";

pub async fn dispatch(args: AuditArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        AuditCommand::Query(a) => query(a, ctx).await,
        AuditCommand::Export(a) => export(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AuditRow {
    audit_id: i64,
    ts: String,
    tenant_id: String,
    mode: String,
    sentence_hash: String,
    label: Option<String>,
    confidence: Option<f32>,
    duration_ms: u64,
    converged: Option<bool>,
}

async fn query(a: AuditQueryArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    if a.limit > 1000 {
        return Err(AdminError::invalid("--limit must be ≤ 1000 (use export for more)"));
    }
    let mut payload = json!({"tenant_id": a.tenant, "limit": a.limit});
    if let Some(s) = &a.since {
        payload["since"] = json!(s);
    }
    if let Some(m) = a.mode {
        payload["mode"] = json!(m);
    }
    let resp = ctx.mcp().call_tool(TOOL_QUERY, payload).await?;
    let rows: Vec<AuditRow> = serde_json::from_value(
        resp.get("rows").cloned().unwrap_or_else(|| serde_json::Value::Array(vec![])),
    )?;
    ctx.renderer.print(&rows)
}

async fn export(a: AuditExportArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    if a.to.exists() {
        return Err(AdminError::invalid(format!(
            "destination already exists: {}",
            a.to.display()
        )));
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_EXPORT,
            json!({"tenant_id": a.tenant, "since": a.since, "format": "jsonl"}),
        )
        .await?;
    let rows = resp
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    write_jsonl(&a.to, &rows).await?;
    eprintln!("✅ {} rows exported to {}", rows.len(), a.to.display());
    Ok(())
}

async fn write_jsonl(path: &PathBuf, rows: &[serde_json::Value]) -> Result<(), AdminError> {
    let f = File::create(path).await?;
    let mut w = BufWriter::new(f);
    for row in rows {
        let line = serde_json::to_string(row)?;
        w.write_all(line.as_bytes()).await?;
        w.write_all(b"\n").await?;
    }
    w.flush().await?;
    Ok(())
}
