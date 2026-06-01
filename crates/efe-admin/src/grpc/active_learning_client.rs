//! REST client for the ActiveLearningService (M24).
//!
//! Same pattern as `curation_client.rs` (M23) — REST gateway over the
//! existing efe-server endpoint. Until full gRPC integration lands,
//! this client wraps the `/v1/learning/*` REST surface.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::auth::TokenProvider;
use crate::context::AdminContext;
use crate::error::AdminError;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ActiveLearningClient {
    base_url: String,
    http: reqwest::Client,
    auth: TokenProvider,
}

impl ActiveLearningClient {
    pub async fn connect(ctx: &AdminContext) -> Result<Self, AdminError> {
        let base_url = ctx.config.cluster_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("efe-admin-al/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AdminError::Network(e.to_string()))?;
        Ok(Self { base_url, http, auth: ctx.auth.clone() })
    }

    fn url(&self, path: &str) -> String { format!("{}/v1/learning{}", self.base_url, path) }

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

    // ── Endpoints ──────────────────────────────────────────────────

    pub async fn list_queue(
        &self, catalog: Option<&str>, status: Option<&str>, limit: u32,
    ) -> Result<Vec<QueueItem>, AdminError> {
        let mut path = format!("/queue?limit={limit}");
        if let Some(c) = catalog { path.push_str(&format!("&catalog={c}")); }
        if let Some(s) = status { path.push_str(&format!("&status={s}")); }
        self.get_json(&path).await
    }

    pub async fn confirm(
        &self, proposal_id: &str, rationale: &str, edit_class_id: Option<&str>,
    ) -> Result<ConfirmResponse, AdminError> {
        if let Some(c) = edit_class_id {
            self.post_json(
                &format!("/proposals/{proposal_id}/edit-and-confirm"),
                &serde_json::json!({"final_class_id": c, "rationale": rationale}),
            ).await
        } else {
            self.post_json(
                &format!("/proposals/{proposal_id}/confirm"),
                &serde_json::json!({"rationale": rationale}),
            ).await
        }
    }

    pub async fn reject(&self, proposal_id: &str, rationale: &str)
        -> Result<ConfirmResponse, AdminError>
    {
        self.post_json(
            &format!("/proposals/{proposal_id}/reject"),
            &serde_json::json!({"rationale": rationale}),
        ).await
    }

    pub async fn trigger_retune(
        &self, catalog: &str, force_below_threshold: bool,
    ) -> Result<TriggerRetuneResponse, AdminError> {
        self.post_json(
            "/retune/trigger",
            &serde_json::json!({
                "catalog_id": catalog,
                "force_below_threshold": force_below_threshold,
            }),
        ).await
    }

    pub async fn list_retunes(&self, catalog: Option<&str>, limit: u32)
        -> Result<Vec<RetuneRunSummary>, AdminError>
    {
        let mut path = format!("/retune?limit={limit}");
        if let Some(c) = catalog { path.push_str(&format!("&catalog={c}")); }
        self.get_json(&path).await
    }

    pub async fn get_retune(&self, run_id: &str) -> Result<RetuneRunDetail, AdminError> {
        self.get_json(&format!("/retune/{run_id}")).await
    }

    pub async fn promote(&self, run_id: &str, skip_canary: bool)
        -> Result<PromoteResponse, AdminError>
    {
        self.post_json(
            &format!("/retune/{run_id}/promote"),
            &serde_json::json!({"skip_canary": skip_canary}),
        ).await
    }

    pub async fn rollback(&self, run_id: &str, reason: &str) -> Result<String, AdminError> {
        let resp: SimpleVersionId = self.post_json(
            &format!("/retune/{run_id}/rollback"),
            &serde_json::json!({"reason": reason}),
        ).await?;
        Ok(resp.restored_active_version_id)
    }

    pub async fn submit_gdpr_erasure(
        &self, catalog: &str, urgency: &str, content_hashes: &[String],
        subject_id: Option<&str>, reason: &str,
    ) -> Result<GdprResponse, AdminError> {
        self.post_json(
            "/gdpr/erasure",
            &serde_json::json!({
                "catalog_id": catalog, "urgency": urgency,
                "content_hashes": content_hashes,
                "requester_subject_id": subject_id, "reason": reason,
            }),
        ).await
    }

    pub async fn list_gdpr_pending(&self) -> Result<Vec<GdprPendingItem>, AdminError> {
        self.get_json("/gdpr/erasure?status=pending").await
    }

    pub async fn get_stats(&self, catalog: Option<&str>) -> Result<Stats, AdminError> {
        let mut path = "/stats".to_string();
        if let Some(c) = catalog { path.push_str(&format!("?catalog={c}")); }
        self.get_json(&path).await
    }

    pub async fn register_dpo_consent(
        &self, signer: &str, addendum_hash: &str,
        signed_at: DateTime<Utc>, expires_at: DateTime<Utc>,
    ) -> Result<DpoConsentResponse, AdminError> {
        self.post_json(
            "/dpo-consent",
            &serde_json::json!({
                "signer_name": signer, "addendum_hash": addendum_hash,
                "signed_at": signed_at, "expires_at": expires_at,
            }),
        ).await
    }
}

// ─── DTOs ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct QueueItem {
    pub id: String,
    pub catalog_id: String,
    pub status: String,
    pub proposed_class_id: Option<String>,
    pub observed_confidence: Option<f64>,
    pub enqueued_at: DateTime<Utc>,
    #[serde(default)]
    pub failure_count: i32,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmResponse {
    pub review_id: String,
    #[serde(default)]
    pub decision: String,
}

#[derive(Debug, Deserialize)]
pub struct TriggerRetuneResponse {
    pub retune_run_id: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RetuneRunSummary {
    pub id: String,
    pub catalog_id: String,
    pub new_bundle_semver: String,
    pub training_examples_count: i64,
    pub holdout_examples_count: i64,
    pub status: String,
    pub flagged_for_promotion: bool,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RetuneRunDetail {
    #[serde(flatten)]
    pub summary: RetuneRunSummary,
    pub eval_metrics: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct PromoteResponse {
    pub new_active_taxonomy_version_id: String,
    pub canary_started_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SimpleVersionId { restored_active_version_id: String }

#[derive(Debug, Deserialize)]
pub struct GdprResponse {
    pub rebuild_request_id: String,
    pub sla_deadline: DateTime<Utc>,
    pub affected_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct GdprPendingItem {
    pub id: String,
    pub catalog_id: String,
    pub urgency: String,
    pub sla_deadline: DateTime<Utc>,
    pub affected_count: u32,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct Stats {
    pub queue_enqueued: u64,
    pub queue_labeling: u64,
    pub queue_awaiting_review: u64,
    pub queue_failed: u64,
    pub oldest_open_age_seconds: i64,
    pub total_gold_examples: u64,
    pub total_holdout_examples: u64,
    pub retune_runs_last_30d: u64,
    pub promotions_last_30d: u64,
    pub cost_cents_today: u64,
    pub cost_cents_last_30d: u64,
}

#[derive(Debug, Deserialize)]
pub struct DpoConsentResponse {
    pub signed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
