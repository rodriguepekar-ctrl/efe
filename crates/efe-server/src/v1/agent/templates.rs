//! Template endpoints under `/v1/agent/templates/*`.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct UpsertTemplateInput {
    pub tenant_id: String,
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub profile: String,
    pub default_provider: String,
    pub default_model: String,
    pub system_prompt: String,
    pub tool_whitelist: Vec<String>,
    #[serde(default = "default_max_steps")]
    pub max_steps: i32,
    #[serde(default = "default_timeout")]
    pub default_timeout_seconds: i32,
    #[serde(default = "default_cost_cents")]
    pub default_max_cost_cents: i64,
    #[serde(default = "default_true")]
    pub require_approval_for_writes: bool,
    pub created_by: String,
    #[serde(default)]
    pub min_channel_required: Option<String>,
}

fn default_max_steps() -> i32 {
    25
}
fn default_timeout() -> i32 {
    1800
}
fn default_cost_cents() -> i64 {
    50
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ListTemplatesQuery {
    pub tenant: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TemplateRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub profile: String,
    pub default_provider: String,
    pub default_model: String,
    pub system_prompt: String,
    pub tool_whitelist_json: serde_json::Value,
    pub max_steps: i32,
    pub default_timeout_seconds: i32,
    pub default_max_cost_cents: i64,
    pub require_approval_for_writes: bool,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub version: i32,
    pub deprecated: bool,
}

pub async fn upsert(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<UpsertTemplateInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    auth.require_tenant(&body.tenant_id)?;

    if body.slug.is_empty() || body.slug.len() > 64 {
        return Err(AgentApiError::InvalidInput(
            "slug must be 1..=64 chars".into(),
        ));
    }
    if !body
        .slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AgentApiError::InvalidInput(
            "slug must match [A-Za-z0-9_-]".into(),
        ));
    }
    if body.system_prompt.is_empty() || body.system_prompt.len() > 32768 {
        return Err(AgentApiError::InvalidInput(
            "system_prompt must be 1..=32768 chars".into(),
        ));
    }
    if body.tool_whitelist.is_empty() {
        return Err(AgentApiError::InvalidInput(
            "tool_whitelist must be non-empty".into(),
        ));
    }
    if !(1..=200).contains(&body.max_steps) {
        return Err(AgentApiError::InvalidInput(
            "max_steps must be 1..=200".into(),
        ));
    }
    if !(60..=14400).contains(&body.default_timeout_seconds) {
        return Err(AgentApiError::InvalidInput(
            "default_timeout_seconds must be 60..=14400".into(),
        ));
    }
    if body.default_max_cost_cents < 1 {
        return Err(AgentApiError::InvalidInput(
            "default_max_cost_cents must be ≥ 1".into(),
        ));
    }

    let prior_version: Option<i32> = sqlx::query_scalar(
        r#"SELECT MAX(version) FROM agent_run_template
           WHERE tenant_id = $1 AND slug = $2"#,
    )
    .bind(&body.tenant_id)
    .bind(&body.slug)
    .fetch_optional(&state.pg)
    .await?
    .flatten();
    let version = prior_version.unwrap_or(0) + 1;

    let id = Uuid::new_v4();
    let now = chrono::Utc::now();
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
    .bind(&body.tenant_id)
    .bind(&body.slug)
    .bind(&body.display_name)
    .bind(&body.description)
    .bind(&body.profile)
    .bind(&body.default_provider)
    .bind(&body.default_model)
    .bind(&body.system_prompt)
    .bind(serde_json::to_value(&body.tool_whitelist)?)
    .bind(body.max_steps)
    .bind(body.default_timeout_seconds)
    .bind(body.default_max_cost_cents)
    .bind(body.require_approval_for_writes)
    .bind(&body.created_by)
    .bind(now)
    .bind(version)
    .execute(&state.pg)
    .await?;

    Ok(Json(serde_json::json!({
        "id": id,
        "slug": body.slug,
        "version": version,
        "deprecated": false,
    })))
}

pub async fn list(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<ListTemplatesQuery>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&q.tenant)?;

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
    .bind(&q.tenant)
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(serde_json::to_value(&rows)?))
}

pub async fn get_one(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path((tenant, slug)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&tenant)?;

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
    .bind(&tenant)
    .bind(&slug)
    .fetch_optional(&state.pg)
    .await?;

    match row {
        Some(r) => Ok(Json(serde_json::to_value(&r)?)),
        None => Err(AgentApiError::NotFound),
    }
}

pub async fn deprecate(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path((tenant, slug)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    auth.require_tenant(&tenant)?;

    let updated = sqlx::query(
        r#"UPDATE agent_run_template
           SET deprecated = TRUE, updated_at = NOW()
           WHERE tenant_id = $1 AND slug = $2"#,
    )
    .bind(&tenant)
    .bind(&slug)
    .execute(&state.pg)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AgentApiError::NotFound);
    }
    Ok(Json(
        serde_json::json!({"success": true, "deprecated": true}),
    ))
}
