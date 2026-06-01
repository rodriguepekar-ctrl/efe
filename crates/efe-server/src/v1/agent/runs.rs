//! Run-lifecycle endpoints under `/v1/agent/runs/*`.

use axum::{
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use uuid::Uuid;

use super::{auth::AuthCtx, errors::AgentApiError, AgentApiState};

// ── DTOs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateRunInput {
    pub tenant_id: String,
    pub template_slug: String,
    pub initial_prompt: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_whitelist: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    pub tenant: String,
    #[serde(default)]
    pub include_completed: bool,
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    50
}

#[derive(Debug, Deserialize)]
pub struct PauseInput {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TerminateInput {
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct StepsQuery {
    #[serde(default)]
    pub since: i32,
    #[serde(default = "default_step_limit")]
    pub limit: i32,
}

fn default_step_limit() -> i32 {
    100
}

// ── Handlers ─────────────────────────────────────────────────────────

pub async fn create(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Json(body): Json<CreateRunInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    auth.require_tenant(&body.tenant_id)?;

    let req = serde_json::to_value(&body)?;
    let run = state.service.create_run(&auth, req).await?;
    Ok(Json(run))
}

pub async fn list(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    auth.require_tenant(&q.tenant)?;

    let runs = state
        .service
        .list_runs(&auth, q.include_completed, q.limit)
        .await?;
    Ok(Json(runs))
}

pub async fn get_one(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    let detail = state.service.get_run(&auth, id).await?;
    Ok(Json(detail))
}

pub async fn pause(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
    Json(body): Json<PauseInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    state.service.pause_run(&auth, id, body.reason).await?;
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn resume(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    state.service.resume_run(&auth, id).await?;
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn terminate(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
    Json(body): Json<TerminateInput>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["operator", "admin"])?;
    if body.reason.trim().len() < 10 {
        return Err(AgentApiError::InvalidInput(
            "reason must be ≥ 10 chars".into(),
        ));
    }
    state.service.terminate_run(&auth, id, body.reason).await?;
    Ok(Json(serde_json::json!({"success": true})))
}

pub async fn list_steps(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
    Query(q): Query<StepsQuery>,
) -> Result<Json<serde_json::Value>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;
    let detail = state.service.get_run(&auth, id).await?;

    // Filter the embedded recent_steps to those past `since`.
    let steps = detail
        .get("recent_steps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let filtered: Vec<_> = steps
        .into_iter()
        .filter(|s| {
            s.get("step_index")
                .and_then(|v| v.as_i64())
                .map(|x| x as i32 > q.since)
                .unwrap_or(false)
        })
        .take(q.limit as usize)
        .collect();
    let terminal = detail
        .get("run")
        .and_then(|r| r.get("status"))
        .and_then(|s| s.as_str())
        .map(|s| {
            matches!(
                s,
                "COMPLETED" | "FAILED" | "TERMINATED" | "TIMED_OUT" | "COST_CAPPED"
            )
        })
        .unwrap_or(false);
    let terminal_status = detail
        .get("run")
        .and_then(|r| r.get("status"))
        .and_then(|s| s.as_str())
        .map(String::from);

    Ok(Json(serde_json::json!({
        "steps": filtered,
        "terminal": terminal,
        "terminal_status": terminal_status,
    })))
}

/// SSE stream of agent steps for a single run.
///
/// Tick interval : 500 ms. Emits `agent_step` events for each new step
/// observed and a terminal `run_terminal` event when the run reaches a
/// terminal status. Closes the stream after the terminal event.
pub async fn stream_steps(
    State(state): State<AgentApiState>,
    auth: AuthCtx,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AgentApiError> {
    auth.require_any_role(&["viewer", "operator", "admin"])?;

    let stream = async_stream::stream! {
        let mut last_index = -1i32;
        let mut idle_iters = 0u32;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let detail = match state.service.get_run(&auth, id).await {
                Ok(d) => d,
                Err(e) => {
                    yield Ok(Event::default()
                        .event("error")
                        .json_data(serde_json::json!({"error": e.to_string()}))
                        .unwrap_or_else(|_| Event::default()));
                    break;
                }
            };

            let mut emitted = 0;
            if let Some(steps) = detail.get("recent_steps").and_then(|v| v.as_array()) {
                for step in steps {
                    let idx = step
                        .get("step_index")
                        .and_then(|v| v.as_i64())
                        .map(|i| i as i32)
                        .unwrap_or(-1);
                    if idx > last_index {
                        last_index = idx;
                        emitted += 1;
                        yield Ok(Event::default()
                            .event("agent_step")
                            .json_data(step.clone())
                            .unwrap_or_else(|_| Event::default()));
                    }
                }
            }

            let status = detail
                .get("run")
                .and_then(|r| r.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if matches!(
                status,
                "COMPLETED" | "FAILED" | "TERMINATED" | "TIMED_OUT" | "COST_CAPPED"
            ) {
                yield Ok(Event::default()
                    .event("run_terminal")
                    .json_data(serde_json::json!({"status": status}))
                    .unwrap_or_else(|_| Event::default()));
                break;
            }

            if emitted == 0 {
                idle_iters += 1;
                if idle_iters > 60 * 30 {
                    // 30 min of no events ; close the stream so the client reconnects.
                    yield Ok(Event::default()
                        .event("stream_idle_timeout")
                        .json_data(serde_json::json!({}))
                        .unwrap_or_else(|_| Event::default()));
                    break;
                }
            } else {
                idle_iters = 0;
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_step_limit_is_100() {
        assert_eq!(default_step_limit(), 100);
    }

    #[test]
    fn default_run_list_limit_is_50() {
        assert_eq!(default_limit(), 50);
    }
}
