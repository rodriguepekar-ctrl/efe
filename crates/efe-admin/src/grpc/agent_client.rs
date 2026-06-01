//! REST client for the AgentRuntimeService (M25).
//!
//! Same pattern as `active_learning_client.rs` (M24). Wraps the
//! `/v1/agent/*` REST surface exposed by efe-server.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::auth::TokenProvider;
use crate::context::AdminContext;
use crate::error::AdminError;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct AgentClient {
    base_url: String,
    http: reqwest::Client,
    auth: TokenProvider,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateAgentRunArgs {
    pub tenant_id: String,
    pub template_slug: String,
    pub initial_prompt: JsonValue,
    pub tool_whitelist: Option<Vec<String>>,
    pub timeout_seconds: Option<i32>,
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentRunDto {
    pub id: String,
    pub tenant_id: String,
    pub template_slug: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub current_step: i32,
    pub max_steps: i32,
    pub current_cost_cents: i64,
    pub max_cost_cents: i64,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentStepDto {
    pub id: String,
    pub step_index: i32,
    pub kind: String,
    pub cost_cents: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApprovalDto {
    pub id: String,
    pub step_index: i32,
    pub tool_name: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentRunDetailDto {
    pub run: AgentRunDto,
    pub recent_steps: Vec<AgentStepDto>,
    pub pending_approvals: Vec<ApprovalDto>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApprovalTokenDto {
    pub approval_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TemplateDto {
    pub slug: String,
    pub version: i32,
    pub deprecated: bool,
    pub profile: String,
    pub default_provider: String,
    pub default_model: String,
    pub max_steps: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CostDailyRow {
    pub day: String,
    pub cents: i64,
    pub runs: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CostPerRun {
    pub total_cents: i64,
    pub llm_calls: i32,
    pub llm_cents: i64,
    pub tool_calls: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DpiaStatusDto {
    pub signed_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub addendum_hash: Option<String>,
    pub is_valid: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DpiaListRow {
    pub tenant: String,
    pub is_valid: bool,
    pub signed_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl AgentClient {
    pub async fn connect(ctx: &AdminContext) -> Result<Self, AdminError> {
        let base_url = ctx.config.cluster_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("efe-admin-agent/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AdminError::Network(e.to_string()))?;
        Ok(Self { base_url, http, auth: ctx.auth.clone() })
    }

    fn url(&self, path: &str) -> String { format!("{}/v1/agent{}", self.base_url, path) }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, AdminError> {
        let mut req = self.http.get(self.url(path));
        if let Some(t) = self.auth.bearer()? { req = req.bearer_auth(t); }
        let resp = req.send().await.map_err(|e| AdminError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AdminError::backend(
                format!("http_{}", resp.status().as_u16()),
                resp.text().await.unwrap_or_default(),
            ));
        }
        resp.json().await.map_err(|e| AdminError::Network(format!("parse: {e}")))
    }

    async fn post_json<B: Serialize, T: serde::de::DeserializeOwned>(
        &self, path: &str, body: &B,
    ) -> Result<T, AdminError> {
        let mut req = self.http.post(self.url(path)).json(body);
        if let Some(t) = self.auth.bearer()? { req = req.bearer_auth(t); }
        let resp = req.send().await.map_err(|e| AdminError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AdminError::backend(
                format!("http_{}", resp.status().as_u16()),
                resp.text().await.unwrap_or_default(),
            ));
        }
        resp.json().await.map_err(|e| AdminError::Network(format!("parse: {e}")))
    }

    // ── Run CRUD ────────────────────────────────────────────────────

    pub async fn create_agent_run(&self, args: CreateAgentRunArgs) -> Result<AgentRunDto, AdminError> {
        self.post_json("/runs", &args).await
    }

    pub async fn list_agent_runs(
        &self,
        tenant: &str,
        all: bool,
        limit: i32,
    ) -> Result<Vec<AgentRunDto>, AdminError> {
        self.get_json(&format!(
            "/runs?tenant={tenant}&include_completed={all}&limit={limit}",
        )).await
    }

    pub async fn get_agent_run(&self, run_id: &str) -> Result<AgentRunDetailDto, AdminError> {
        self.get_json(&format!("/runs/{run_id}")).await
    }

    pub async fn pause_agent_run(&self, run_id: &str, reason: Option<&str>) -> Result<(), AdminError> {
        let body = serde_json::json!({"reason": reason});
        let _: serde_json::Value = self.post_json(&format!("/runs/{run_id}/pause"), &body).await?;
        Ok(())
    }

    pub async fn resume_agent_run(&self, run_id: &str) -> Result<(), AdminError> {
        let body = serde_json::json!({});
        let _: serde_json::Value = self.post_json(&format!("/runs/{run_id}/resume"), &body).await?;
        Ok(())
    }

    pub async fn terminate_agent_run(&self, run_id: &str, reason: &str) -> Result<(), AdminError> {
        let body = serde_json::json!({"reason": reason});
        let _: serde_json::Value = self.post_json(&format!("/runs/{run_id}/terminate"), &body).await?;
        Ok(())
    }

    // ── Approval ───────────────────────────────────────────────────

    pub async fn approve_agent_step(
        &self,
        approval_id: &str,
        reason: &str,
    ) -> Result<ApprovalTokenDto, AdminError> {
        let body = serde_json::json!({"approval_id": approval_id, "reason": reason});
        self.post_json("/approvals/approve", &body).await
    }

    pub async fn reject_agent_step(
        &self,
        approval_id: &str,
        reason: &str,
    ) -> Result<(), AdminError> {
        let body = serde_json::json!({"approval_id": approval_id, "reason": reason});
        let _: serde_json::Value = self.post_json("/approvals/reject", &body).await?;
        Ok(())
    }

    // ── Templates ───────────────────────────────────────────────────

    pub async fn list_agent_run_templates(&self, tenant: &str) -> Result<Vec<TemplateDto>, AdminError> {
        self.get_json(&format!("/templates?tenant={tenant}")).await
    }

    pub async fn get_agent_run_template(
        &self,
        tenant: &str,
        slug: &str,
    ) -> Result<TemplateDto, AdminError> {
        self.get_json(&format!("/templates/{tenant}/{slug}")).await
    }

    pub async fn upsert_agent_run_template(
        &self,
        body: serde_yaml::Value,
    ) -> Result<TemplateDto, AdminError> {
        let json: JsonValue = serde_json::to_value(body)
            .map_err(|e| AdminError::InvalidInput(format!("yaml→json : {e}")))?;
        self.post_json("/templates", &json).await
    }

    // ── Cost ────────────────────────────────────────────────────────

    pub async fn cost_daily(
        &self,
        tenant: &str,
        days: i32,
    ) -> Result<Vec<CostDailyRow>, AdminError> {
        self.get_json(&format!("/cost/daily?tenant={tenant}&days={days}")).await
    }

    pub async fn cost_per_run(&self, run_id: &str) -> Result<CostPerRun, AdminError> {
        self.get_json(&format!("/cost/run/{run_id}")).await
    }

    // ── DPIA ────────────────────────────────────────────────────────

    pub async fn dpia_sign(
        &self,
        tenant: &str,
        addendum_hash: &str,
        valid_days: i64,
    ) -> Result<(), AdminError> {
        let body = serde_json::json!({
            "tenant_id": tenant,
            "addendum_hash": addendum_hash,
            "valid_days": valid_days,
        });
        let _: serde_json::Value = self.post_json("/dpia/sign", &body).await?;
        Ok(())
    }

    pub async fn dpia_status(&self, tenant: &str) -> Result<DpiaStatusDto, AdminError> {
        self.get_json(&format!("/dpia/status?tenant={tenant}")).await
    }

    pub async fn dpia_list(&self) -> Result<Vec<DpiaListRow>, AdminError> {
        self.get_json("/dpia/list").await
    }

    pub async fn dpia_refresh(&self, tenant: &str, valid_days: i64) -> Result<(), AdminError> {
        let body = serde_json::json!({"tenant_id": tenant, "valid_days": valid_days});
        let _: serde_json::Value = self.post_json("/dpia/refresh", &body).await?;
        Ok(())
    }

    // ── M26 graduation ──────────────────────────────────────────────

    pub async fn graduate_check(&self, tenant: &str) -> Result<GraduateCheckDto, AdminError> {
        let body = serde_json::json!({"tenant_id": tenant});
        self.post_json("/graduate/check", &body).await
    }

    pub async fn graduate_confirm(
        &self,
        tenant: &str,
        acknowledgement_hash: &str,
    ) -> Result<GraduateConfirmDto, AdminError> {
        let body = serde_json::json!({
            "tenant_id": tenant,
            "acknowledgement_hash": acknowledgement_hash,
        });
        self.post_json("/graduate/confirm", &body).await
    }

    pub async fn graduate_revoke(
        &self,
        tenant: &str,
        reason: &str,
    ) -> Result<GraduateRevokeDto, AdminError> {
        let body = serde_json::json!({"tenant_id": tenant, "reason": reason});
        self.post_json("/graduate/revoke", &body).await
    }

    pub async fn graduate_status(&self, tenant: &str) -> Result<GraduateStatusDto, AdminError> {
        // Status piggybacks on /graduate/check (read-only, returns channel + dates).
        let check = self.graduate_check(tenant).await?;
        let dpia: DpiaStatusDto = self.dpia_status(tenant).await.unwrap_or(DpiaStatusDto {
            signed_at: None,
            expires_at: None,
            addendum_hash: None,
            is_valid: false,
        });
        Ok(GraduateStatusDto {
            channel: check.channel,
            graduated_at: None, // not in /check ; re-fetch from /confirm if needed
            revoked_at: None,
            dpia_expires_at: dpia.expires_at,
        })
    }
}

// ── M26 graduation DTOs ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GraduateCheckDto {
    pub tenant_id: String,
    pub channel: String,
    pub operational: GraduateOperationalDto,
    pub compliance: GraduateComplianceDto,
    pub ready: bool,
    pub blocking_items: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct GraduateOperationalDto {
    pub days_in_experimental: i32,
    pub min_days_required: i32,
    pub successful_runs: i64,
    pub min_runs_required: i64,
    pub sandbox_escapes_90d: i64,
    pub failure_rate_pct_30d: f64,
    pub max_failure_rate_pct: f64,
    pub dpia_valid: bool,
    pub graduation_acknowledgement_signed: bool,
}

#[derive(Debug, Deserialize)]
pub struct GraduateComplianceDto {
    pub audit_chain_verified: bool,
    pub r100_pack_succeeded: bool,
    pub dpo_review_current_quarter: bool,
    pub ai_act_post_market_submitted: bool,
}

#[derive(Debug, Deserialize)]
pub struct GraduateConfirmDto {
    pub success: bool,
    pub graduated_at: DateTime<Utc>,
    pub acknowledgement_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct GraduateRevokeDto {
    pub success: bool,
    pub revoked_at: DateTime<Utc>,
    pub channel: String,
}

#[derive(Debug)]
pub struct GraduateStatusDto {
    pub channel: String,
    pub graduated_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub dpia_expires_at: Option<DateTime<Utc>>,
}
