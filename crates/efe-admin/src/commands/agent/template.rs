//! `efe-admin agent template` — list / get / upsert AgentRunTemplates.

use clap::Subcommand;
use std::path::PathBuf;

use crate::context::AdminContext;
use crate::error::AdminError;

#[derive(Debug, Subcommand)]
pub enum TemplateCmd {
    /// List templates for tenant.
    List {
        #[arg(long)] tenant: String,
    },
    /// Get a single template by slug.
    Get {
        #[arg(long)] tenant: String,
        slug: String,
    },
    /// Upsert a template from a YAML file (bumps version on collision).
    Upsert {
        #[arg(long)] file: String,
        #[arg(long, default_value_t = false)] confirm: bool,
    },
}

pub async fn dispatch(c: TemplateCmd, ctx: &AdminContext) -> Result<(), AdminError> {
    match c {
        TemplateCmd::List { tenant } => list(tenant, ctx).await,
        TemplateCmd::Get { tenant, slug } => get(tenant, slug, ctx).await,
        TemplateCmd::Upsert { file, confirm } => upsert(file, confirm, ctx).await,
    }
}

async fn list(tenant: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let tmpls = crate::grpc::agent_client::AgentClient::connect(ctx).await?.list_agent_run_templates(&tenant).await?;
    if tmpls.is_empty() {
        println!("(no templates for tenant '{}')", tenant);
        return Ok(());
    }
    println!("{:<28}  {:<12}  {:<14}  {:>3}  {:<24}", "slug", "version", "profile", "max", "model");
    for t in tmpls {
        println!(
            "{:<28}  {:<12}  {:<14}  {:>3}  {:<24}",
            t.slug,
            format!("v{}{}", t.version, if t.deprecated { "(d)" } else { "" }),
            t.profile,
            t.max_steps,
            format!("{}/{}", t.default_provider, t.default_model),
        );
    }
    Ok(())
}

async fn get(tenant: String, slug: String, ctx: &AdminContext) -> Result<(), AdminError> {
    let t = crate::grpc::agent_client::AgentClient::connect(ctx).await?.get_agent_run_template(&tenant, &slug).await?;
    println!("{}", serde_yaml::to_string(&t).map_err(|e| AdminError::InvalidInput(e.to_string()))?);
    Ok(())
}

async fn upsert(file: String, confirm: bool, ctx: &AdminContext) -> Result<(), AdminError> {
    if !confirm {
        return Err(AdminError::InvalidInput(
            "must pass --confirm to upsert a template".into(),
        ));
    }
    let raw = std::fs::read_to_string(PathBuf::from(&file))
        .map_err(|e| AdminError::InvalidInput(format!("read {file} : {e}")))?;
    let t: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|e| AdminError::InvalidInput(format!("parse YAML : {e}")))?;
    let res = crate::grpc::agent_client::AgentClient::connect(ctx).await?.upsert_agent_run_template(t).await?;
    println!("✓ Template upserted : slug={} version={}", res.slug, res.version);
    Ok(())
}
