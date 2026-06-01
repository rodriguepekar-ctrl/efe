//! D93 (M26 assembly) — adapter that bridges the JSON-typed
//! `AgentRuntimeFacade` trait (REST handlers) to the typed
//! `AgentRuntimeService` (M25 in-process service).
//!
//! The trait's signatures use `serde_json::Value` because the REST
//! handlers serialize back through serde_json. The adapter parses the
//! JSON into typed `CreateAgentRunRequest` etc. and translates errors.

use std::sync::Arc;

use efe_agent_runner::approval::ApprovalToken as RunnerApprovalToken;
use efe_agent_runner::service::{AgentRuntimeService, CallerIdentity, CreateAgentRunRequest};
use efe_agent_runner::{AgentRunnerError, TenantId};
use uuid::Uuid;

use super::auth::AuthCtx;
use super::errors::AgentApiError;
use super::AgentRuntimeFacade;

/// Wraps the in-process `AgentRuntimeService` and converts its typed
/// surface to the JSON-typed `AgentRuntimeFacade` trait that the
/// axum REST handlers expect.
pub struct AgentRuntimeServiceAdapter {
    pub inner: Arc<AgentRuntimeService>,
}

impl AgentRuntimeServiceAdapter {
    pub fn new(inner: Arc<AgentRuntimeService>) -> Self {
        Self { inner }
    }
}

fn auth_to_caller(auth: &AuthCtx) -> CallerIdentity {
    CallerIdentity {
        subject: auth.subject.clone(),
        tenant: TenantId(auth.tenant.clone()),
        roles: auth.roles.clone(),
        token_age_seconds: auth.token_age_seconds,
    }
}

#[async_trait::async_trait]
impl AgentRuntimeFacade for AgentRuntimeServiceAdapter {
    async fn create_run(
        &self,
        caller: &AuthCtx,
        req: serde_json::Value,
    ) -> Result<serde_json::Value, AgentApiError> {
        let typed: CreateAgentRunRequest = serde_json::from_value(req).map_err(|e| {
            AgentApiError::InvalidInput(format!("CreateAgentRunRequest parse: {}", e))
        })?;
        let inner_caller = auth_to_caller(caller);
        let run = self
            .inner
            .create_agent_run(&inner_caller, typed)
            .await
            .map_err(map_runner_err)?;
        Ok(serde_json::to_value(&run)?)
    }

    async fn get_run(
        &self,
        caller: &AuthCtx,
        run_id: Uuid,
    ) -> Result<serde_json::Value, AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        let run = self
            .inner
            .get_agent_run(&inner_caller, run_id)
            .await
            .map_err(map_runner_err)?;
        // The recent_steps + pending_approvals are surfaced as separate
        // calls so the legacy `get_agent_run` can stay thin. Errors on
        // those auxiliary lookups don't fail the parent fetch.
        let recent_steps = self
            .inner
            .list_recent_steps(&inner_caller, run_id, 50)
            .await
            .unwrap_or_default();
        let pending_approvals = self
            .inner
            .list_pending_approvals_for_run(&inner_caller, run_id)
            .await
            .unwrap_or_default();
        Ok(serde_json::json!({
            "run": run,
            "recent_steps": recent_steps,
            "pending_approvals": pending_approvals,
        }))
    }

    async fn list_runs(
        &self,
        caller: &AuthCtx,
        _include_completed: bool,
        _limit: i32,
    ) -> Result<serde_json::Value, AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        let runs = self
            .inner
            .list_active_runs(&inner_caller)
            .await
            .map_err(map_runner_err)?;
        Ok(serde_json::to_value(&runs)?)
    }

    async fn pause_run(
        &self,
        caller: &AuthCtx,
        run_id: Uuid,
        reason: Option<String>,
    ) -> Result<(), AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        self.inner
            .pause_agent_run(&inner_caller, run_id, reason)
            .await
            .map_err(map_runner_err)?;
        Ok(())
    }

    async fn resume_run(&self, caller: &AuthCtx, run_id: Uuid) -> Result<(), AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        self.inner
            .resume_agent_run(&inner_caller, run_id)
            .await
            .map_err(map_runner_err)?;
        Ok(())
    }

    async fn terminate_run(
        &self,
        caller: &AuthCtx,
        run_id: Uuid,
        reason: String,
    ) -> Result<(), AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        self.inner
            .terminate_agent_run(&inner_caller, run_id, reason)
            .await
            .map_err(map_runner_err)?;
        Ok(())
    }

    async fn approve_step(
        &self,
        caller: &AuthCtx,
        approval_id: Uuid,
        reason: String,
    ) -> Result<serde_json::Value, AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        let token: RunnerApprovalToken = self
            .inner
            .approve_agent_step(&inner_caller, approval_id, Some(reason))
            .await
            .map_err(map_runner_err)?;
        Ok(serde_json::json!({
            "approval_id": token.approval_id,
            "agent_run_id": token.agent_run_id,
            "step_index": token.step_index,
            "approval_token": format!("{}.{}", token.nonce, token.mac_hex),
            "expires_at": null,
        }))
    }

    async fn reject_step(
        &self,
        caller: &AuthCtx,
        approval_id: Uuid,
        reason: String,
    ) -> Result<(), AgentApiError> {
        let inner_caller = auth_to_caller(caller);
        self.inner
            .reject_agent_step(&inner_caller, approval_id, reason)
            .await
            .map_err(map_runner_err)?;
        Ok(())
    }
}

/// Translate an `AgentRunnerError` to an `AgentApiError` (HTTP-shaped).
fn map_runner_err(e: AgentRunnerError) -> AgentApiError {
    use AgentRunnerError as E;
    match e {
        E::InvalidInput(msg) => AgentApiError::InvalidInput(msg),

        E::RunNotFound(_)
        | E::StepNotFound(_)
        | E::TemplateNotFound(_)
        | E::ApprovalNotFound(_) => AgentApiError::NotFound,

        E::M25ServerDisabled
        | E::M25NotEnabledForTenant(_)
        | E::M21Required
        | E::CrdMissing
        | E::KvmUnavailable
        | E::DpiaNotSigned(_)
        | E::DpiaExpired { .. }
        | E::DpiaHashMismatch { .. }
        | E::InvalidTransition { .. }
        | E::RunAlreadyTerminated(_)
        | E::NotResumable(_)
        | E::ToolNotInWhitelist { .. }
        | E::WriteToolRequiresApproval { .. }
        | E::InvalidApprovalToken
        | E::ApprovalTokenExpired
        | E::ImageNotInWhitelist(_)
        | E::ProfileNotAllowed(_)
        | E::ProfileUnknown(_)
        | E::ProviderNotAllowed(_) => AgentApiError::FailedPrecondition(e.to_string()),

        E::PermissionDenied { .. } | E::TenantMismatch { .. } => {
            AgentApiError::Forbidden(e.to_string())
        }

        E::OidcTokenStale { age_seconds, max } => AgentApiError::TokenStale { age_seconds, max },

        E::CostCapExceeded { .. }
        | E::CostAboveTenantCap { .. }
        | E::MaxConcurrentRunsReached { .. } => AgentApiError::RateLimited {
            retry_after_seconds: 60,
        },

        E::MicroVmProvisioningFailed(_) | E::MicroVmNotFound(_) => AgentApiError::Unavailable,

        // CRITICAL — never leak details. A correlation-ID-only error
        // is returned to the caller while the full record goes to
        // tracing + PagerDuty.
        E::SandboxEscapeAttempt { .. } => AgentApiError::SandboxEscape,

        E::AuditChainBroken { .. }
        | E::AuditHmacFailure(_)
        | E::Database(_)
        | E::Json(_)
        | E::Kube(_)
        | E::Internal(_) => AgentApiError::Internal(e.to_string()),
    }
}
