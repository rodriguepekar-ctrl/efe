//! gRPC service implementation for the Agent Runtime API.
//!
//! Wires the public API surface (CreateAgentRun, GetAgentRun,
//! ApproveAgentStep, RejectAgentStep, PauseAgentRun, ResumeAgentRun,
//! TerminateAgentRun, ListAgentRuns) to the internal modules.
//!
//! Authorization : every method requires the caller's OIDC subject +
//! tenant ; we cross-check `caller.tenant == request.tenant`.

use crate::approval::{self, ApprovalToken, CreateApprovalInput};
use crate::audit::{self, AgentAuditEvent};
use crate::cost;
use crate::profile::ProfileRegistry;
use crate::run::{self, AgentRun, AgentRunStatus, CreateRunInput};
use crate::template;
use crate::tool_acl::ToolWhitelist;
use crate::{AgentRunnerError, AgentRunnerResult, TenantAgentRuntimeFlags, TenantId};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Caller identity (extracted from a verified OIDC bearer by an
/// upstream interceptor).
#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub subject: String,
    pub tenant: TenantId,
    pub roles: Vec<String>,
    pub token_age_seconds: i64,
}

impl CallerIdentity {
    pub fn requires_tenant(&self, tenant: &TenantId) -> AgentRunnerResult<()> {
        if self.tenant.0 != tenant.0 {
            return Err(AgentRunnerError::TenantMismatch {
                caller: self.tenant.0.clone(),
                requested: tenant.0.clone(),
            });
        }
        Ok(())
    }

    pub fn requires_role(&self, action: &str, allowed: &[&str]) -> AgentRunnerResult<()> {
        let has = self.roles.iter().any(|r| allowed.iter().any(|a| a == r));
        if !has {
            return Err(AgentRunnerError::PermissionDenied {
                role: self.roles.join(","),
                action: action.into(),
            });
        }
        Ok(())
    }

    pub fn requires_fresh_token(&self, max_age: i64) -> AgentRunnerResult<()> {
        if self.token_age_seconds > max_age {
            return Err(AgentRunnerError::OidcTokenStale {
                age_seconds: self.token_age_seconds,
                max: max_age,
            });
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait TenantFlagsProvider: Send + Sync {
    async fn flags_for(&self, tenant: &TenantId) -> AgentRunnerResult<TenantAgentRuntimeFlags>;
}

/// In-process service. Holds dependencies + dispatches ; no networking.
pub struct AgentRuntimeService {
    pub pg: PgPool,
    pub flags: Arc<dyn TenantFlagsProvider>,
    pub profiles: Arc<ProfileRegistry>,
}

impl AgentRuntimeService {
    pub fn new(
        pg: PgPool,
        flags: Arc<dyn TenantFlagsProvider>,
        profiles: Arc<ProfileRegistry>,
    ) -> Self {
        Self { pg, flags, profiles }
    }

    // ── CreateAgentRun ──────────────────────────────────────────────

    pub async fn create_agent_run(
        &self,
        caller: &CallerIdentity,
        req: CreateAgentRunRequest,
    ) -> AgentRunnerResult<AgentRun> {
        caller.requires_tenant(&req.tenant_id)?;
        caller.requires_role("agent_run.create", &["operator", "admin"])?;
        caller.requires_fresh_token(300)?;

        // Tenant flags + DPIA validation.
        let flags = self.flags.flags_for(&req.tenant_id).await?;
        if !flags.agent_runtime_enabled {
            return Err(AgentRunnerError::M25NotEnabledForTenant(req.tenant_id.0.clone()));
        }
        if !flags.dpia_valid() {
            return Err(AgentRunnerError::DpiaNotSigned(req.tenant_id.0.clone()));
        }

        // Load template (canonical defaults).
        let tmpl = template::get_active(&self.pg, &req.tenant_id, &req.template_slug).await?;

        // Profile + provider must be tenant-allowed.
        let profile = self.profiles.enforce_tenant_allow(&tmpl.profile, &flags.allowed_profiles)?;
        if !flags.allowed_providers.iter().any(|p| p == &tmpl.default_provider) {
            return Err(AgentRunnerError::ProviderNotAllowed(tmpl.default_provider.clone()));
        }

        // Caller-provided overrides : merge with template + profile.
        let max_cost_usd = req
            .max_cost_usd
            .unwrap_or(profile.default_max_cost_cents as f64 / 100.0);
        if max_cost_usd > flags.cost_cap_per_run_usd {
            return Err(AgentRunnerError::CostAboveTenantCap {
                req: max_cost_usd,
                cap: flags.cost_cap_per_run_usd,
            });
        }
        let max_cost_cents = (max_cost_usd * 100.0).round() as i64;
        let timeout_s = req
            .timeout_seconds
            .unwrap_or(tmpl.default_timeout_seconds);
        if timeout_s as u32 > flags.timeout_minutes_max * 60 {
            return Err(AgentRunnerError::InvalidInput(format!(
                "timeout_seconds {} > tenant cap {}s",
                timeout_s,
                flags.timeout_minutes_max * 60
            )));
        }

        // Concurrency check (cheap pre-check ; the SQL trigger is the
        // enforcement point, but we want a fast path with a clean error).
        let active = run::list_active_runs_for_tenant(&self.pg, &req.tenant_id).await?;
        if active.len() as u32 >= flags.max_concurrent_runs {
            return Err(AgentRunnerError::MaxConcurrentRunsReached {
                tenant: req.tenant_id.0.clone(),
                current: active.len() as u32,
                max: flags.max_concurrent_runs,
            });
        }

        // Tool whitelist : caller may supply a SUBSET of the template's.
        let whitelist_strs = req.tool_whitelist.unwrap_or_else(|| tmpl.tool_whitelist.clone());
        for t in &whitelist_strs {
            if !tmpl.tool_whitelist.iter().any(|x| x == t) {
                return Err(AgentRunnerError::InvalidInput(format!(
                    "tool '{}' not in template whitelist", t
                )));
            }
        }
        let _ = ToolWhitelist::from_strings(&whitelist_strs)?;

        // Insert + audit.
        let mut tx = self.pg.begin().await?;
        let agent_run = run::insert_run(&mut tx, &CreateRunInput {
            tenant_id: req.tenant_id.clone(),
            template_id: tmpl.slug.clone(),
            profile: tmpl.profile.clone(),
            provider: tmpl.default_provider.clone(),
            model: tmpl.default_model.clone(),
            created_by: caller.subject.clone(),
            timeout_seconds: timeout_s,
            max_cost_cents,
            max_steps: tmpl.max_steps,
            initial_prompt: req.initial_prompt,
            tool_whitelist: whitelist_strs.clone(),
        }).await?;

        audit::append_agent_audit(
            &mut tx,
            &req.tenant_id,
            Some(agent_run.id),
            None,
            AgentAuditEvent::RunCreated,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({
                "template_slug": tmpl.slug,
                "profile": tmpl.profile,
                "tool_whitelist": whitelist_strs,
                "max_cost_cents": max_cost_cents,
                "timeout_seconds": timeout_s,
            }),
        ).await?;
        tx.commit().await?;

        crate::metrics::AGENT_RUNS_TOTAL
            .with_label_values(&[&req.tenant_id.0, "PENDING"])
            .inc();
        crate::metrics::AGENT_RUNS_ACTIVE
            .with_label_values(&[&req.tenant_id.0])
            .inc();

        Ok(agent_run)
    }

    // ── GetAgentRun ────────────────────────────────────────────────

    pub async fn get_agent_run(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
    ) -> AgentRunnerResult<AgentRun> {
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        caller.requires_role("agent_run.read", &["viewer", "operator", "admin"])?;
        Ok(r)
    }

    // ── ApproveAgentStep ───────────────────────────────────────────

    pub async fn approve_agent_step(
        &self,
        caller: &CallerIdentity,
        approval_id: Uuid,
        reason: Option<String>,
    ) -> AgentRunnerResult<ApprovalToken> {
        caller.requires_role("agent_step.approve", &["operator", "admin"])?;
        caller.requires_fresh_token(300)?;
        let mut tx = self.pg.begin().await?;
        let token = approval::grant(&mut tx, approval_id, &caller.subject, reason.as_deref()).await?;
        audit::append_agent_audit(
            &mut tx,
            &caller.tenant,
            Some(token.agent_run_id),
            None,
            AgentAuditEvent::ApprovalGranted,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({"approval_id": approval_id, "step_index": token.step_index}),
        ).await?;
        tx.commit().await?;
        Ok(token)
    }

    pub async fn reject_agent_step(
        &self,
        caller: &CallerIdentity,
        approval_id: Uuid,
        reason: String,
    ) -> AgentRunnerResult<()> {
        caller.requires_role("agent_step.reject", &["operator", "admin"])?;
        let mut tx = self.pg.begin().await?;
        approval::reject(&mut tx, approval_id, &caller.subject, &reason).await?;
        audit::append_agent_audit(
            &mut tx,
            &caller.tenant,
            None,
            None,
            AgentAuditEvent::ApprovalRejected,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({"approval_id": approval_id, "reason": reason}),
        ).await?;
        tx.commit().await?;
        Ok(())
    }

    // ── Pause / Resume ─────────────────────────────────────────────

    pub async fn pause_agent_run(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
        reason: Option<String>,
    ) -> AgentRunnerResult<()> {
        caller.requires_role("agent_run.pause", &["operator", "admin"])?;
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        if r.status != AgentRunStatus::Running {
            return Err(AgentRunnerError::InvalidTransition {
                from: r.status.as_sql_str().into(),
                to: "PAUSED".into(),
            });
        }
        let mut tx = self.pg.begin().await?;
        run::update_status(&mut tx, run_id, AgentRunStatus::Paused).await?;
        audit::append_agent_audit(
            &mut tx,
            &r.tenant_id,
            Some(run_id),
            None,
            AgentAuditEvent::PauseRequested,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({"reason": reason}),
        ).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn resume_agent_run(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
    ) -> AgentRunnerResult<()> {
        caller.requires_role("agent_run.resume", &["operator", "admin"])?;
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        if r.status != AgentRunStatus::Paused {
            return Err(AgentRunnerError::NotResumable(r.status.as_sql_str().into()));
        }
        let mut tx = self.pg.begin().await?;
        run::update_status(&mut tx, run_id, AgentRunStatus::Resuming).await?;
        audit::append_agent_audit(
            &mut tx,
            &r.tenant_id,
            Some(run_id),
            None,
            AgentAuditEvent::RunResumed,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({}),
        ).await?;
        tx.commit().await?;
        Ok(())
    }

    // ── Terminate ──────────────────────────────────────────────────

    pub async fn terminate_agent_run(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
        reason: String,
    ) -> AgentRunnerResult<()> {
        caller.requires_role("agent_run.terminate", &["operator", "admin"])?;
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        if r.status.is_terminal() {
            return Err(AgentRunnerError::RunAlreadyTerminated(run_id));
        }
        let mut tx = self.pg.begin().await?;
        run::update_status(&mut tx, run_id, AgentRunStatus::Terminated).await?;
        audit::append_agent_audit(
            &mut tx,
            &r.tenant_id,
            Some(run_id),
            None,
            AgentAuditEvent::RunTerminated,
            &caller.subject,
            Some(&caller.roles.join(",")),
            serde_json::json!({"reason": reason}),
        ).await?;
        tx.commit().await?;
        crate::metrics::AGENT_RUNS_TOTAL
            .with_label_values(&[&r.tenant_id.0, "TERMINATED"])
            .inc();
        crate::metrics::AGENT_RUNS_ACTIVE
            .with_label_values(&[&r.tenant_id.0])
            .dec();
        Ok(())
    }

    // ── List ───────────────────────────────────────────────────────

    pub async fn list_active_runs(
        &self,
        caller: &CallerIdentity,
    ) -> AgentRunnerResult<Vec<AgentRun>> {
        caller.requires_role("agent_run.read", &["viewer", "operator", "admin"])?;
        run::list_active_runs_for_tenant(&self.pg, &caller.tenant).await
    }

    // ── Internal: CreateApprovalRequest (called by mcp-server, not user) ─

    pub async fn create_approval_request(
        &self,
        tenant: &TenantId,
        agent_run_id: Uuid,
        step_index: i32,
        tool_name: String,
        tool_args: serde_json::Value,
        justification: Option<String>,
        ttl_seconds: i64,
    ) -> AgentRunnerResult<crate::approval::ApprovalRequest> {
        let mut tx = self.pg.begin().await?;
        let ar = approval::create(&mut tx, &CreateApprovalInput {
            agent_run_id,
            tenant_id: tenant.clone(),
            step_index,
            tool_name: tool_name.clone(),
            tool_args_json: tool_args.clone(),
            justification: justification.clone(),
            ttl_seconds,
        }).await?;
        run::update_status(&mut tx, agent_run_id, AgentRunStatus::AwaitingApproval).await?;
        audit::append_agent_audit(
            &mut tx,
            tenant,
            Some(agent_run_id),
            None,
            AgentAuditEvent::ApprovalRequested,
            "system:agent-core",
            Some("system"),
            serde_json::json!({
                "approval_id": ar.id,
                "tool_name": tool_name,
                "step_index": step_index,
            }),
        ).await?;
        tx.commit().await?;
        Ok(ar)
    }

    // ── Internal: ReportFinalOutput ────────────────────────────────

    pub async fn report_final_output(
        &self,
        tenant: &TenantId,
        agent_run_id: Uuid,
        final_output: serde_json::Value,
    ) -> AgentRunnerResult<()> {
        let mut tx = self.pg.begin().await?;
        let r = run::get_run(&self.pg, agent_run_id).await?;
        if r.tenant_id.0 != tenant.0 {
            return Err(AgentRunnerError::TenantMismatch {
                caller: tenant.0.clone(),
                requested: r.tenant_id.0.clone(),
            });
        }
        let _ = sqlx::query(
            r#"UPDATE agent_run SET final_output_json = $1 WHERE id = $2"#,
        )
        .bind(&final_output)
        .bind(agent_run_id)
        .execute(&mut *tx)
        .await?;
        run::update_status(&mut tx, agent_run_id, AgentRunStatus::Completed).await?;
        audit::append_agent_audit(
            &mut tx,
            tenant,
            Some(agent_run_id),
            None,
            AgentAuditEvent::FinalOutput,
            "system:agent-core",
            Some("system"),
            serde_json::json!({"len": final_output.to_string().len()}),
        ).await?;
        tx.commit().await?;

        let started = r.started_at.unwrap_or_else(Utc::now);
        let secs = (Utc::now() - started).num_seconds().max(0) as f64;
        crate::metrics::AGENT_RUN_DURATION
            .with_label_values(&[&tenant.0, "COMPLETED"])
            .observe(secs);
        crate::metrics::AGENT_RUNS_TOTAL
            .with_label_values(&[&tenant.0, "COMPLETED"])
            .inc();
        crate::metrics::AGENT_RUNS_ACTIVE
            .with_label_values(&[&tenant.0])
            .dec();
        Ok(())
    }

    // ── Internal: cost helper ──────────────────────────────────────

    pub async fn add_cost(
        &self,
        tenant: &TenantId,
        agent_run_id: Uuid,
        delta_cents: i64,
    ) -> AgentRunnerResult<i64> {
        let mut tx = self.pg.begin().await?;
        let total = cost::add_cost(&mut tx, agent_run_id, delta_cents).await?;
        crate::metrics::AGENT_COST_CENTS_TOTAL
            .with_label_values(&[&tenant.0])
            .inc_by(delta_cents.max(0) as u64);
        tx.commit().await?;
        Ok(total)
    }

    // ── D93 (M26 assembly) — methods used by the REST gateway adapter ──

    /// Return the most recent N steps for a run (newest last).
    pub async fn list_recent_steps(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
        limit: i32,
    ) -> AgentRunnerResult<Vec<serde_json::Value>> {
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        caller.requires_role("agent_run.read", &["viewer", "operator", "admin"])?;
        let limit = limit.clamp(1, 500);
        let rows: Vec<StepRow> = sqlx::query_as(
            r#"SELECT id, step_index, kind, tool_name, payload_json,
                      cost_cents, duration_ms, created_at, completed_at, status
               FROM agent_step
               WHERE agent_run_id = $1
               ORDER BY step_index ASC
               LIMIT $2"#,
        )
        .bind(run_id)
        .bind(limit as i64)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
            .collect())
    }

    /// Pending approvals for a single run (any status PENDING + not expired).
    pub async fn list_pending_approvals_for_run(
        &self,
        caller: &CallerIdentity,
        run_id: Uuid,
    ) -> AgentRunnerResult<Vec<serde_json::Value>> {
        let r = run::get_run(&self.pg, run_id).await?;
        caller.requires_tenant(&r.tenant_id)?;
        caller.requires_role("agent_run.read", &["viewer", "operator", "admin"])?;
        let rows: Vec<ApprovalRow> = sqlx::query_as(
            r#"SELECT id, agent_run_id, tenant_id, step_index, tool_name,
                      tool_args_json, justification, status, requested_at,
                      expires_at, decided_at, decided_by, decision_reason
               FROM agent_approval_request
               WHERE agent_run_id = $1
                 AND status = 'PENDING'::agent_approval_status
                 AND expires_at > NOW()
               ORDER BY requested_at ASC"#,
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
            .collect())
    }
}

#[derive(sqlx::FromRow, Serialize)]
struct StepRow {
    id: Uuid,
    step_index: i32,
    kind: String,
    tool_name: Option<String>,
    payload_json: Option<serde_json::Value>,
    cost_cents: i64,
    duration_ms: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    status: String,
}

#[derive(sqlx::FromRow, Serialize)]
struct ApprovalRow {
    id: Uuid,
    agent_run_id: Uuid,
    tenant_id: String,
    step_index: i32,
    tool_name: String,
    tool_args_json: serde_json::Value,
    justification: Option<String>,
    status: crate::approval::ApprovalStatus,
    requested_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    decided_at: Option<chrono::DateTime<chrono::Utc>>,
    decided_by: Option<String>,
    decision_reason: Option<String>,
}

// ── DTOs (gRPC-facing) ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRunRequest {
    pub tenant_id: TenantId,
    pub template_slug: String,
    pub initial_prompt: serde_json::Value,
    pub tool_whitelist: Option<Vec<String>>,
    pub timeout_seconds: Option<i32>,
    pub max_cost_usd: Option<f64>,
    pub created_by: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_role_mismatch_rejected() {
        let caller = CallerIdentity {
            subject: "u1".into(),
            tenant: TenantId("acme".into()),
            roles: vec!["viewer".into()],
            token_age_seconds: 30,
        };
        let err = caller.requires_role("agent_run.create", &["operator", "admin"]).unwrap_err();
        assert!(matches!(err, AgentRunnerError::PermissionDenied { .. }));
    }

    #[test]
    fn caller_tenant_mismatch_rejected() {
        let caller = CallerIdentity {
            subject: "u1".into(),
            tenant: TenantId("acme".into()),
            roles: vec!["operator".into()],
            token_age_seconds: 30,
        };
        let err = caller.requires_tenant(&TenantId("beta".into())).unwrap_err();
        assert!(matches!(err, AgentRunnerError::TenantMismatch { .. }));
    }

    #[test]
    fn caller_stale_token_rejected() {
        let caller = CallerIdentity {
            subject: "u1".into(),
            tenant: TenantId("acme".into()),
            roles: vec!["operator".into()],
            token_age_seconds: 600,
        };
        let err = caller.requires_fresh_token(300).unwrap_err();
        assert!(matches!(err, AgentRunnerError::OidcTokenStale { .. }));
    }
}
