//! DPIA endpoints under `/v1/agent/dpia/*`.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct SignInput {
    pub tenant_id: String,
    pub addendum_hash: String,
    #[serde(default = "default_valid_days")]
    pub valid_days: i64,
}

fn default_valid_days() -> i64 {
    365
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub tenant: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshInput {
    pub tenant_id: String,
    #[serde(default = "default_valid_days")]
    pub valid_days: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StatusDto {
    pub signed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub addendum_hash: Option<String>,
    pub is_valid: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ListRow {
    pub tenant: String,
    pub is_valid: bool,
    pub signed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn sign(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<SignInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_role("admin")?;
    auth.require_tenant(&body.tenant_id)?;

    if !body.addendum_hash.starts_with("sha256:") || body.addendum_hash.len() != 71 {
        return Err(AgentApiError::InvalidInput(
            "addendum_hash must be 'sha256:<64-hex>'".into(),
        ));
    }
    if !(30..=730).contains(&body.valid_days) {
        return Err(AgentApiError::InvalidInput(
            "valid_days must be 30..=730".into(),
        ));
    }

    let updated = sqlx::query(
        r#"UPDATE tenant_feature
           SET m25_dpia_signed_at = NOW(),
               m25_dpia_expires_at = NOW() + ($1 || ' days')::interval,
               m25_dpia_addendum_hash = $2
           WHERE id = $3"#,
    )
    .bind(body.valid_days.to_string())
    .bind(&body.addendum_hash)
    .bind(&body.tenant_id)
    .execute(&state.pg)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AgentApiError::NotFound);
    }
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn status(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<StatusQuery>,
) -> Result<Json<StatusDto>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&q.tenant)?;

    let dto: Option<StatusDto> = sqlx::query_as(
        r#"SELECT
              m25_dpia_signed_at AS signed_at,
              m25_dpia_expires_at AS expires_at,
              m25_dpia_addendum_hash AS addendum_hash,
              (m25_dpia_signed_at IS NOT NULL AND m25_dpia_expires_at > NOW()) AS is_valid
           FROM tenant_feature
           WHERE id = $1"#,
    )
    .bind(&q.tenant)
    .fetch_optional(&state.pg)
    .await?;

    match dto {
        Some(d) => Ok(Json(d)),
        None => Err(AgentApiError::NotFound),
    }
}

pub async fn list(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
) -> Result<Json<Vec<ListRow>>, AgentApiError> {
    auth.require_role("admin")?;

    let rows: Vec<ListRow> = sqlx::query_as(
        r#"SELECT
              id AS tenant,
              (m25_dpia_signed_at IS NOT NULL AND m25_dpia_expires_at > NOW()) AS is_valid,
              m25_dpia_signed_at AS signed_at,
              m25_dpia_expires_at AS expires_at
           FROM tenant_feature
           WHERE m25_agent_runtime_enabled = TRUE
           ORDER BY id ASC"#,
    )
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(rows))
}

pub async fn refresh(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<RefreshInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_role("admin")?;
    auth.require_tenant(&body.tenant_id)?;

    if !(30..=730).contains(&body.valid_days) {
        return Err(AgentApiError::InvalidInput(
            "valid_days must be 30..=730".into(),
        ));
    }

    let updated = sqlx::query(
        r#"UPDATE tenant_feature
           SET m25_dpia_signed_at = NOW(),
               m25_dpia_expires_at = NOW() + ($1 || ' days')::interval
           WHERE id = $2 AND m25_dpia_addendum_hash IS NOT NULL"#,
    )
    .bind(body.valid_days.to_string())
    .bind(&body.tenant_id)
    .execute(&state.pg)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AgentApiError::FailedPrecondition(
            "tenant has no prior DPIA addendum_hash to refresh against".into(),
        ));
    }
    Ok(Json(serde_json::json!({"success": true})))
}
