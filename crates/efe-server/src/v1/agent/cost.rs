//! Cost reporting endpoints under `/v1/agent/cost/*`.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

#[derive(Debug, Deserialize)]
pub struct DailyQuery {
    pub tenant: String,
    #[serde(default = "default_days")]
    pub days: i32,
}

fn default_days() -> i32 {
    7
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DailyRow {
    pub day: chrono::NaiveDate,
    pub cents: i64,
    pub runs: i32,
}

pub async fn daily(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<DailyQuery>,
) -> Result<Json<Vec<DailyDto>>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&q.tenant)?;

    if !(1..=90).contains(&q.days) {
        return Err(AgentApiError::InvalidInput("days must be 1..=90".into()));
    }

    let rows: Vec<DailyRow> = sqlx::query_as(
        r#"SELECT
              date_trunc('day', created_at)::date AS day,
              COALESCE(SUM(current_cost_cents), 0)::bigint AS cents,
              COUNT(*)::int AS runs
           FROM agent_run
           WHERE tenant_id = $1
             AND created_at >= NOW() - ($2 || ' days')::interval
           GROUP BY 1
           ORDER BY 1 DESC"#,
    )
    .bind(&q.tenant)
    .bind(q.days.to_string())
    .fetch_all(&state.pg)
    .await?;

    let dto: Vec<DailyDto> = rows
        .into_iter()
        .map(|r| DailyDto {
            day: r.day.to_string(),
            cents: r.cents,
            runs: r.runs,
        })
        .collect();
    Ok(Json(dto))
}

#[derive(Debug, Serialize)]
pub struct DailyDto {
    pub day: String,
    pub cents: i64,
    pub runs: i32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PerRunDto {
    pub total_cents: i64,
    pub llm_calls: i32,
    pub llm_cents: i64,
    pub tool_calls: i32,
}

pub async fn per_run(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(run_id): Path<Uuid>,
) -> Result<Json<PerRunDto>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;

    let dto: Option<PerRunDto> = sqlx::query_as(
        r#"SELECT
              COALESCE(SUM(cost_cents), 0)::bigint AS total_cents,
              COUNT(*) FILTER (WHERE kind = 'llm_call')::int AS llm_calls,
              COALESCE(SUM(cost_cents) FILTER (WHERE kind = 'llm_call'), 0)::bigint AS llm_cents,
              COUNT(*) FILTER (WHERE kind = 'tool_call')::int AS tool_calls
           FROM agent_step
           WHERE agent_run_id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(&state.pg)
    .await?;

    Ok(Json(dto.unwrap_or(PerRunDto {
        total_cents: 0,
        llm_calls: 0,
        llm_cents: 0,
        tool_calls: 0,
    })))
}
