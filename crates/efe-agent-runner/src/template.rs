//! `agent_run_template` — reusable parameterized agent run definition.
//!
//! Operators define templates via the K8s `AgentRunTemplate` CRD or
//! the gRPC service ; an `AgentRun` is then created by *instantiating*
//! a template (with parameter overrides). Templates are versioned and
//! per-tenant.

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunTemplate {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub slug: String,                    // e.g. "research_lite_v1"
    pub display_name: String,
    pub description: Option<String>,
    pub profile: String,
    pub default_provider: String,
    pub default_model: String,
    pub system_prompt: String,
    pub tool_whitelist: Vec<String>,
    pub max_steps: i32,
    pub default_timeout_seconds: i32,
    pub default_max_cost_cents: i64,
    pub require_approval_for_writes: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
    pub deprecated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertTemplateInput {
    pub tenant_id: TenantId,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub profile: String,
    pub default_provider: String,
    pub default_model: String,
    pub system_prompt: String,
    pub tool_whitelist: Vec<String>,
    pub max_steps: i32,
    pub default_timeout_seconds: i32,
    pub default_max_cost_cents: i64,
    pub require_approval_for_writes: bool,
    pub created_by: String,
}

impl UpsertTemplateInput {
    pub fn validate(&self) -> AgentRunnerResult<()> {
        if self.slug.is_empty() || self.slug.len() > 64 {
            return Err(AgentRunnerError::InvalidInput(
                "slug must be 1..=64 chars".into(),
            ));
        }
        if !self
            .slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(AgentRunnerError::InvalidInput(
                "slug must be ascii [a-zA-Z0-9_-]".into(),
            ));
        }
        if self.system_prompt.is_empty() {
            return Err(AgentRunnerError::InvalidInput(
                "system_prompt cannot be empty".into(),
            ));
        }
        if self.tool_whitelist.is_empty() {
            return Err(AgentRunnerError::InvalidInput(
                "tool_whitelist cannot be empty (use ['noop'] for read-only agent)".into(),
            ));
        }
        if self.max_steps < 1 || self.max_steps > 200 {
            return Err(AgentRunnerError::InvalidInput(
                "max_steps must be 1..=200".into(),
            ));
        }
        if self.default_timeout_seconds < 60 || self.default_timeout_seconds > 14400 {
            return Err(AgentRunnerError::InvalidInput(
                "default_timeout_seconds must be 60..=14400 (4h)".into(),
            ));
        }
        if self.default_max_cost_cents < 1 {
            return Err(AgentRunnerError::InvalidInput(
                "default_max_cost_cents must be ≥ 1".into(),
            ));
        }
        Ok(())
    }
}

pub async fn upsert(
    tx: &mut Transaction<'_, Postgres>,
    input: &UpsertTemplateInput,
) -> AgentRunnerResult<AgentRunTemplate> {
    input.validate()?;
    // Bump version on slug collision.
    let prior_version: Option<i32> = sqlx::query_scalar(
        r#"SELECT MAX(version) FROM agent_run_template
           WHERE tenant_id = $1 AND slug = $2"#,
    )
    .bind(&input.tenant_id.0)
    .bind(&input.slug)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    let version = prior_version.unwrap_or(0) + 1;

    let id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        r#"INSERT INTO agent_run_template
           (id, tenant_id, slug, display_name, description,
            profile, default_provider, default_model, system_prompt,
            tool_whitelist_json, max_steps, default_timeout_seconds,
            default_max_cost_cents, require_approval_for_writes,
            created_by, created_at, updated_at, version, deprecated)
           VALUES ($1, $2, $3, $4, $5,
                   $6, $7, $8, $9,
                   $10, $11, $12,
                   $13, $14,
                   $15, $16, $16, $17, FALSE)"#,
    )
    .bind(id)
    .bind(&input.tenant_id.0)
    .bind(&input.slug)
    .bind(&input.display_name)
    .bind(&input.description)
    .bind(&input.profile)
    .bind(&input.default_provider)
    .bind(&input.default_model)
    .bind(&input.system_prompt)
    .bind(serde_json::to_value(&input.tool_whitelist)?)
    .bind(input.max_steps)
    .bind(input.default_timeout_seconds)
    .bind(input.default_max_cost_cents)
    .bind(input.require_approval_for_writes)
    .bind(&input.created_by)
    .bind(now)
    .bind(version)
    .execute(&mut **tx)
    .await?;
    Ok(AgentRunTemplate {
        id,
        tenant_id: input.tenant_id.clone(),
        slug: input.slug.clone(),
        display_name: input.display_name.clone(),
        description: input.description.clone(),
        profile: input.profile.clone(),
        default_provider: input.default_provider.clone(),
        default_model: input.default_model.clone(),
        system_prompt: input.system_prompt.clone(),
        tool_whitelist: input.tool_whitelist.clone(),
        max_steps: input.max_steps,
        default_timeout_seconds: input.default_timeout_seconds,
        default_max_cost_cents: input.default_max_cost_cents,
        require_approval_for_writes: input.require_approval_for_writes,
        created_by: input.created_by.clone(),
        created_at: now,
        updated_at: now,
        version,
        deprecated: false,
    })
}

pub async fn get_active(
    pg: &PgPool,
    tenant: &TenantId,
    slug: &str,
) -> AgentRunnerResult<AgentRunTemplate> {
    let row: Option<TemplateRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, slug, display_name, description,
                  profile, default_provider, default_model, system_prompt,
                  tool_whitelist_json, max_steps, default_timeout_seconds,
                  default_max_cost_cents, require_approval_for_writes,
                  created_by, created_at, updated_at, version, deprecated
           FROM agent_run_template
           WHERE tenant_id = $1 AND slug = $2 AND NOT deprecated
           ORDER BY version DESC LIMIT 1"#,
    )
    .bind(&tenant.0)
    .bind(slug)
    .fetch_optional(pg)
    .await?;
    row.map(Into::into)
        .ok_or_else(|| AgentRunnerError::TemplateNotFound(slug.to_string()))
}

pub async fn list(
    pg: &PgPool,
    tenant: &TenantId,
) -> AgentRunnerResult<Vec<AgentRunTemplate>> {
    let rows: Vec<TemplateRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, slug, display_name, description,
                  profile, default_provider, default_model, system_prompt,
                  tool_whitelist_json, max_steps, default_timeout_seconds,
                  default_max_cost_cents, require_approval_for_writes,
                  created_by, created_at, updated_at, version, deprecated
           FROM agent_run_template
           WHERE tenant_id = $1
           ORDER BY slug ASC, version DESC"#,
    )
    .bind(&tenant.0)
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct TemplateRow {
    id: Uuid,
    tenant_id: String,
    slug: String,
    display_name: String,
    description: Option<String>,
    profile: String,
    default_provider: String,
    default_model: String,
    system_prompt: String,
    tool_whitelist_json: serde_json::Value,
    max_steps: i32,
    default_timeout_seconds: i32,
    default_max_cost_cents: i64,
    require_approval_for_writes: bool,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    version: i32,
    deprecated: bool,
}

impl From<TemplateRow> for AgentRunTemplate {
    fn from(r: TemplateRow) -> Self {
        Self {
            id: r.id,
            tenant_id: TenantId(r.tenant_id),
            slug: r.slug,
            display_name: r.display_name,
            description: r.description,
            profile: r.profile,
            default_provider: r.default_provider,
            default_model: r.default_model,
            system_prompt: r.system_prompt,
            tool_whitelist: serde_json::from_value(r.tool_whitelist_json).unwrap_or_default(),
            max_steps: r.max_steps,
            default_timeout_seconds: r.default_timeout_seconds,
            default_max_cost_cents: r.default_max_cost_cents,
            require_approval_for_writes: r.require_approval_for_writes,
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            version: r.version,
            deprecated: r.deprecated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_input() -> UpsertTemplateInput {
        UpsertTemplateInput {
            tenant_id: TenantId("acme".into()),
            slug: "research_lite".into(),
            display_name: "Research lite".into(),
            description: None,
            profile: "research_lite".into(),
            default_provider: "openai".into(),
            default_model: "gpt-4o-mini".into(),
            system_prompt: "You are a research assistant.".into(),
            tool_whitelist: vec!["web_search".into(), "fetch_url".into()],
            max_steps: 25,
            default_timeout_seconds: 1800,
            default_max_cost_cents: 50,
            require_approval_for_writes: true,
            created_by: "ops@acme".into(),
        }
    }

    #[test]
    fn validate_ok() {
        assert!(ok_input().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_whitelist() {
        let mut i = ok_input();
        i.tool_whitelist.clear();
        assert!(i.validate().is_err());
    }

    #[test]
    fn validate_rejects_long_slug() {
        let mut i = ok_input();
        i.slug = "x".repeat(70);
        assert!(i.validate().is_err());
    }

    #[test]
    fn validate_rejects_max_steps_above_200() {
        let mut i = ok_input();
        i.max_steps = 1000;
        assert!(i.validate().is_err());
    }

    #[test]
    fn validate_rejects_timeout_too_long() {
        let mut i = ok_input();
        i.default_timeout_seconds = 86400;
        assert!(i.validate().is_err());
    }
}
