//! `efe-admin catalog <subcommand>` — bundle seeding + listing + promotion.

use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::fs::File;

use crate::cli::{CatalogArgs, CatalogCommand, CatalogListArgs, CatalogPromoteArgs, CatalogSeedArgs};
use crate::context::AdminContext;
use crate::error::AdminError;
use crate::prompts::confirm;
use crate::validation;

// `catalog seed` is the new D8 tool. `catalog list` reuses M16.
// `catalog promote` reuses the M18 bundle-promotion tool.
const TOOL_SEED: &str = "efe_seed_catalog";
const TOOL_LIST: &str = "efe_list_catalogs";
const TOOL_PROMOTE: &str = "efe_promote_bundle";

pub async fn dispatch(args: CatalogArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    match args.command {
        CatalogCommand::Seed(a) => seed(a, ctx).await,
        CatalogCommand::List(a) => list(a, ctx).await,
        CatalogCommand::Promote(a) => promote(a, ctx).await,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SeedOutput {
    tenant_id: String,
    catalog: String,
    bundle_version: String,
    anchors_seeded: u32,
    duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnchorRow {
    anchor_id: String,
    text: String,
    label: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

async fn seed(a: CatalogSeedArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    validation::semver(&a.bundle_version)?;

    if !a.bundle.exists() {
        return Err(AdminError::invalid(format!(
            "bundle file not found: {}",
            a.bundle.display()
        )));
    }

    // Validate JSONL upfront (cheap, catches typos before going over wire).
    let anchors = parse_jsonl(&a.bundle).await?;
    if anchors.is_empty() {
        return Err(AdminError::invalid("bundle is empty"));
    }
    eprintln!("📦 bundle parsed: {} anchors", anchors.len());

    if !ctx.confirm_bypass()
        && !confirm(
            &format!(
                "Seed catalog '{}' for tenant '{}' (bundle_version={}, {} anchors) ?",
                a.catalog, a.tenant, a.bundle_version, anchors.len()
            ),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }

    let pb = ProgressBar::new(anchors.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {pos:>5}/{len:5} {wide_bar:.cyan/blue} {msg}",
        )
        .expect("template")
        .progress_chars("█░"),
    );
    pb.set_message("seeding…");

    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_SEED,
            json!({
                "tenant_id": a.tenant,
                "catalog": a.catalog,
                "bundle_version": a.bundle_version,
                "anchors": anchors,
                "confirm": true,
            }),
        )
        .await?;

    pb.finish_and_clear();
    let parsed: SeedOutput = serde_json::from_value(resp)?;
    eprintln!(
        "✅ {} anchors seeded in {} ms",
        parsed.anchors_seeded, parsed.duration_ms
    );
    ctx.renderer.print(&parsed)
}

async fn parse_jsonl(path: &Path) -> Result<Vec<AnchorRow>, AdminError> {
    let f = File::open(path).await?;
    let mut reader = BufReader::new(f).lines();
    let mut out = Vec::with_capacity(1024);
    let mut ln = 0_usize;
    while let Some(line) = reader.next_line().await? {
        ln += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let row: AnchorRow = serde_json::from_str(trimmed).map_err(|e| {
            AdminError::invalid(format!("line {ln}: invalid JSONL — {e}"))
        })?;
        if row.anchor_id.is_empty() || row.text.is_empty() || row.label.is_empty() {
            return Err(AdminError::invalid(format!(
                "line {ln}: anchor_id/text/label cannot be empty"
            )));
        }
        out.push(row);
    }
    Ok(out)
}

#[derive(Debug, Serialize, Deserialize)]
struct CatalogRow {
    catalog: String,
    bundle_version: String,
    anchors: u32,
    promoted_at: String,
    status: String,
}

async fn list(a: CatalogListArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    let resp = ctx.mcp().call_tool(TOOL_LIST, json!({"tenant_id": a.tenant})).await?;
    let rows: Vec<CatalogRow> = serde_json::from_value(
        resp.get("catalogs").cloned().unwrap_or_else(|| serde_json::Value::Array(vec![])),
    )?;
    ctx.renderer.print(&rows)
}

async fn promote(a: CatalogPromoteArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    validation::tenant_slug(&a.tenant)?;
    validation::semver(&a.to_version)?;
    if !ctx.confirm_bypass()
        && !confirm(
            &format!(
                "Promote catalog '{}' for tenant '{}' to version {} ?",
                a.catalog, a.tenant, a.to_version
            ),
            ctx.config.confirm_mode,
        )?
    {
        return Err(AdminError::UserAbort);
    }
    let resp = ctx
        .mcp()
        .call_tool(
            TOOL_PROMOTE,
            json!({
                "tenant_id": a.tenant,
                "catalog": a.catalog,
                "to_version": a.to_version,
                "confirm": true,
            }),
        )
        .await?;
    ctx.renderer.print(&resp)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[tokio::test]
    async fn parse_jsonl_happy_path() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"anchor_id":"a01","text":"facturation tardive","label":"billing"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"anchor_id":"a02","text":"changement adresse","label":"customer-info"}}"#
        )
        .unwrap();
        let res = parse_jsonl(f.path()).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].anchor_id, "a01");
    }

    #[tokio::test]
    async fn parse_jsonl_skips_blank_and_comment() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"# this is a comment"#).unwrap();
        writeln!(f).unwrap();
        writeln!(
            f,
            r#"{{"anchor_id":"a01","text":"hello","label":"x"}}"#
        )
        .unwrap();
        let res = parse_jsonl(f.path()).await.unwrap();
        assert_eq!(res.len(), 1);
    }

    #[tokio::test]
    async fn parse_jsonl_empty_field_errors() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"anchor_id":"","text":"x","label":"y"}}"#).unwrap();
        let res = parse_jsonl(f.path()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn parse_jsonl_bad_json_errors() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"not json at all"#).unwrap();
        let res = parse_jsonl(f.path()).await;
        assert!(res.is_err());
    }
}
