//! Approval endpoints under `/v1/agent/approvals/*`.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct ApproveInput {
    pub approval_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct RejectInput {
    pub approval_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ListPendingQuery {
    pub tenant: String,
}

pub async fn approve(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<ApproveInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    if body.reason.trim().len() < 10 {
        return Err(AgentApiError::InvalidInput(
            "reason must be ≥ 10 chars (Art. 14 audit)".into(),
        ));
    }
    let token = state
        .service
        .approve_step(&auth, body.approval_id, body.reason)
        .await?;
    Ok(Json(token))
}

pub async fn reject(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<RejectInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    if body.reason.trim().len() < 10 {
        return Err(AgentApiError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    state
        .service
        .reject_step(&auth, body.approval_id, body.reason)
        .await?;
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn list_pending(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<ListPendingQuery>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&q.tenant)?;

    let rows: Vec<serde_json::Value> = sqlx::query_as::<_, ApprovalRow>(
        r#"SELECT id, agent_run_id, tenant_id, step_index, tool_name,
                  tool_args_json, justification, requested_at, status,
                  expires_at
           FROM agent_approval_request
           WHERE tenant_id = $1
             AND status = 'PENDING'::agent_approval_status
             AND expires_at > NOW()
           ORDER BY requested_at ASC
           LIMIT 100"#,
    )
    .bind(&q.tenant)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|r| serde_json::to_value(r).unwrap())
    .collect();

    Ok(Json(serde_json::json!({"pending": rows})))
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct ApprovalRow {
    id: Uuid,
    agent_run_id: Uuid,
    tenant_id: String,
    step_index: i32,
    tool_name: String,
    tool_args_json: serde_json::Value,
    justification: Option<String>,
    requested_at: chrono::DateTime<chrono::Utc>,
    status: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}
