//! Agent run record + state machine + persistence.
//!
//! `agent_run` is the central aggregate : one row per LLM-driven
//! autonomous task. State transitions are validated both at the Rust
//! layer (`AgentRunStatus::can_transition_to`) and at the Postgres
//! layer (`fn_agent_run_state_machine` trigger in migration 0016).

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "agent_run_status", rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentRunStatus {
    Pending,
    Provisioning,
    Running,
    AwaitingApproval,
    Paused,
    Resuming,
    Completed,
    Failed,
    Terminated,
    TimedOut,
    CostCapped,
}

impl AgentRunStatus {
    /// Whitelist of allowed transitions. Mirrors the SQL trigger
    /// (`fn_agent_run_state_machine`) verbatim.
    pub fn can_transition_to(&self, next: AgentRunStatus) -> bool {
        use AgentRunStatus::*;
        matches!(
            (self, next),
            (Pending, Provisioning)
            | (Pending, Failed)
            | (Pending, Terminated)
            | (Provisioning, Running)
            | (Provisioning, Failed)
            | (Provisioning, Terminated)
            | (Running, AwaitingApproval)
            | (Running, Paused)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Terminated)
            | (Running, TimedOut)
            | (Running, CostCapped)
            | (AwaitingApproval, Running)
            | (AwaitingApproval, Failed)
            | (AwaitingApproval, Terminated)
            | (AwaitingApproval, TimedOut)
            | (Paused, Resuming)
            | (Paused, Terminated)
            | (Resuming, Running)
            | (Resuming, Failed)
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::Terminated
                | Self::TimedOut
                | Self::CostCapped
        )
    }

    pub fn is_active(&self) -> bool { !self.is_terminal() }

    /// Canonical postgres ENUM string (matches the SQL `agent_run_status`).
    /// Preferred over `format!("{:?}", …)` because Debug omits the
    /// SCREAMING_SNAKE_CASE underscores.
    pub fn as_sql_str(&self) -> &'static str {
        match self {
            Self::Pending           => "PENDING",
            Self::Provisioning      => "PROVISIONING",
            Self::Running           => "RUNNING",
            Self::AwaitingApproval  => "AWAITING_APPROVAL",
            Self::Paused            => "PAUSED",
            Self::Resuming          => "RESUMING",
            Self::Completed         => "COMPLETED",
            Self::Failed            => "FAILED",
            Self::Terminated        => "TERMINATED",
            Self::TimedOut          => "TIMED_OUT",
            Self::CostCapped        => "COST_CAPPED",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub template_id: String,
    pub profile: String,                // e.g. "research_lite"
    pub provider: String,                // e.g. "openai"
    pub model: String,                   // e.g. "gpt-4o-mini"
    pub status: AgentRunStatus,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub timeout_seconds: i32,
    pub max_cost_cents: i64,
    pub current_cost_cents: i64,
    pub current_step: i32,
    pub max_steps: i32,
    pub vm_id: Option<String>,
    pub final_output: Option<serde_json::Value>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunInput {
    pub tenant_id: TenantId,
    pub template_id: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub created_by: String,
    pub timeout_seconds: i32,
    pub max_cost_cents: i64,
    pub max_steps: i32,
    pub initial_prompt: serde_json::Value,
    pub tool_whitelist: Vec<String>,
}

pub async fn insert_run(
    tx: &mut Transaction<'_, Postgres>,
    input: &CreateRunInput,
) -> AgentRunnerResult<AgentRun> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    // D93 — column renamed `template_id` → `template_slug` in migration
    // 0017. The Rust struct field stays `template_id` for caller
    // compatibility ; we alias on SELECT and use the new column on INSERT.
    sqlx::query(
        r#"INSERT INTO agent_run
           (id, tenant_id, template_slug, profile, provider, model,
            status, created_by, created_at,
            timeout_seconds, max_cost_cents, current_cost_cents,
            current_step, max_steps,
            initial_prompt_json, tool_whitelist_json)
           VALUES ($1, $2, $3, $4, $5, $6,
                   'PENDING'::agent_run_status, $7, $8,
                   $9, $10, 0,
                   0, $11,
                   $12, $13)"#,
    )
    .bind(id)
    .bind(&input.tenant_id.0)
    .bind(&input.template_id)
    .bind(&input.profile)
    .bind(&input.provider)
    .bind(&input.model)
    .bind(&input.created_by)
    .bind(now)
    .bind(input.timeout_seconds)
    .bind(input.max_cost_cents)
    .bind(input.max_steps)
    .bind(&input.initial_prompt)
    .bind(serde_json::to_value(&input.tool_whitelist)?)
    .execute(&mut **tx)
    .await?;

    Ok(AgentRun {
        id,
        tenant_id: input.tenant_id.clone(),
        template_id: input.template_id.clone(),
        profile: input.profile.clone(),
        provider: input.provider.clone(),
        model: input.model.clone(),
        status: AgentRunStatus::Pending,
        created_by: input.created_by.clone(),
        created_at: now,
        started_at: None,
        completed_at: None,
        timeout_seconds: input.timeout_seconds,
        max_cost_cents: input.max_cost_cents,
        current_cost_cents: 0,
        current_step: 0,
        max_steps: input.max_steps,
        vm_id: None,
        final_output: None,
        failure_reason: None,
    })
}

pub async fn get_run(pg: &PgPool, run_id: Uuid) -> AgentRunnerResult<AgentRun> {
    let row: Option<AgentRunRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, template_slug AS template_id, profile, provider, model,
                  status, created_by, created_at, started_at, completed_at,
                  timeout_seconds, max_cost_cents, current_cost_cents,
                  current_step, max_steps, vm_id,
                  final_output_json, failure_reason
           FROM agent_run WHERE id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(pg)
    .await?;
    row.map(Into::into).ok_or(AgentRunnerError::RunNotFound(run_id))
}

pub async fn update_status(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    new_status: AgentRunStatus,
) -> AgentRunnerResult<()> {
    let updated = sqlx::query(
        r#"UPDATE agent_run
           SET status = $1::agent_run_status,
               started_at = COALESCE(started_at,
                   CASE WHEN $1::agent_run_status = 'RUNNING' THEN NOW() END),
               completed_at = CASE
                   WHEN $1::agent_run_status IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')
                   THEN NOW() ELSE completed_at END
           WHERE id = $2"#,
    )
    .bind(new_status.as_sql_str())
    .bind(run_id)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AgentRunnerError::RunNotFound(run_id));
    }
    Ok(())
}

pub async fn list_active_runs_for_tenant(
    pg: &PgPool,
    tenant: &TenantId,
) -> AgentRunnerResult<Vec<AgentRun>> {
    let rows: Vec<AgentRunRow> = sqlx::query_as(
        r#"SELECT id, tenant_id, template_slug AS template_id, profile, provider, model,
                  status, created_by, created_at, started_at, completed_at,
                  timeout_seconds, max_cost_cents, current_cost_cents,
                  current_step, max_steps, vm_id,
                  final_output_json, failure_reason
           FROM agent_run
           WHERE tenant_id = $1
             AND status NOT IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')
           ORDER BY created_at DESC"#,
    )
    .bind(&tenant.0)
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct AgentRunRow {
    id: Uuid,
    tenant_id: String,
    template_id: String,
    profile: String,
    provider: String,
    model: String,
    status: AgentRunStatus,
    created_by: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    timeout_seconds: i32,
    max_cost_cents: i64,
    current_cost_cents: i64,
    current_step: i32,
    max_steps: i32,
    vm_id: Option<String>,
    final_output_json: Option<serde_json::Value>,
    failure_reason: Option<String>,
}

impl From<AgentRunRow> for AgentRun {
    fn from(r: AgentRunRow) -> Self {
        Self {
            id: r.id,
            tenant_id: TenantId(r.tenant_id),
            template_id: r.template_id,
            profile: r.profile,
            provider: r.provider,
            model: r.model,
            status: r.status,
            created_by: r.created_by,
            created_at: r.created_at,
            started_at: r.started_at,
            completed_at: r.completed_at,
            timeout_seconds: r.timeout_seconds,
            max_cost_cents: r.max_cost_cents,
            current_cost_cents: r.current_cost_cents,
            current_step: r.current_step,
            max_steps: r.max_steps,
            vm_id: r.vm_id,
            final_output: r.final_output_json,
            failure_reason: r.failure_reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_to_provisioning_allowed() {
        assert!(AgentRunStatus::Pending.can_transition_to(AgentRunStatus::Provisioning));
    }
    #[test]
    fn pending_to_running_forbidden_directly() {
        assert!(!AgentRunStatus::Pending.can_transition_to(AgentRunStatus::Running));
    }
    #[test]
    fn completed_is_sink() {
        assert!(!AgentRunStatus::Completed.can_transition_to(AgentRunStatus::Running));
    }
    #[test]
    fn terminated_is_sink() {
        assert!(!AgentRunStatus::Terminated.can_transition_to(AgentRunStatus::Running));
    }
    #[test]
    fn awaiting_approval_back_to_running() {
        assert!(AgentRunStatus::AwaitingApproval.can_transition_to(AgentRunStatus::Running));
    }
    #[test]
    fn paused_resume_path() {
        assert!(AgentRunStatus::Paused.can_transition_to(AgentRunStatus::Resuming));
        assert!(AgentRunStatus::Resuming.can_transition_to(AgentRunStatus::Running));
        // Paused → Running directly is forbidden (must go via Resuming).
        assert!(!AgentRunStatus::Paused.can_transition_to(AgentRunStatus::Running));
    }
    #[test]
    fn cost_capped_is_terminal() {
        assert!(AgentRunStatus::CostCapped.is_terminal());
    }
}
