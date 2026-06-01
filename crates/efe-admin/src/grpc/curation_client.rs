//! Stub REST client for the curation service used by the CLI.
//!
//! Mirrors the pattern of `grpc::client::GrpcClient` (despite its name,
//! that file uses REST over the existing gateway). Until the curation
//! gRPC endpoints land in efe-server's REST gateway, this module exposes
//! a typed surface that the CLI subcommands call ; the implementation
//! returns `AdminError::Backend("not_implemented", …)` for endpoints
//! that have not yet been wired through.
//!
//! The intent is to keep the CLI surface compilable + testable while the
//! backend integration progresses through D70-D73.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::auth::TokenProvider;
use crate::context::AdminContext;
use crate::error::AdminError;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct CurationClient {
    base_url: String,
    http: reqwest::Client,
    auth: TokenProvider,
}

impl CurationClient {
    /// Connect using the active admin context (cluster URL + auth provider).
    pub async fn connect(ctx: &AdminContext) -> Result<Self, AdminError> {
        let base_url = ctx.config.cluster_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("efe-admin-curation/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AdminError::Network(e.to_string()))?;
        Ok(Self { base_url, http, auth: ctx.auth.clone() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/curation{}", self.base_url, path)
    }

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

    // ── Endpoints ────────────────────────────────────────────────────

    pub async fn list_pending(&self, catalog: Option<&str>, limit: i64)
        -> Result<Vec<ProposalSummary>, AdminError>
    {
        let mut path = format!("/proposals?status=pending&limit={limit}");
        if let Some(c) = catalog { path.push_str(&format!("&catalog={c}")); }
        self.get_json(&path).await
    }

    pub async fn get_proposal(&self, id: &str) -> Result<ProposalDetail, AdminError> {
        self.get_json(&format!("/proposals/{id}")).await
    }

    pub async fn approve_proposal(&self, id: &str, rationale: &str) -> Result<String, AdminError> {
        let resp: SimpleStatus = self.post_json(
            &format!("/proposals/{id}/approve"),
            &serde_json::json!({ "rationale": rationale }),
        ).await?;
        Ok(resp.new_status)
    }

    pub async fn reject_proposal(&self, id: &str, rationale: &str) -> Result<String, AdminError> {
        let resp: SimpleStatus = self.post_json(
            &format!("/proposals/{id}/reject"),
            &serde_json::json!({ "rationale": rationale }),
        ).await?;
        Ok(resp.new_status)
    }

    pub async fn promote_proposal(&self, id: &str, skip_canary: bool)
        -> Result<PromoteResponse, AdminError>
    {
        self.post_json(
            &format!("/proposals/{id}/promote"),
            &serde_json::json!({ "skip_canary": skip_canary }),
        ).await
    }

    pub async fn rollback_version(&self, version_id: &str, reason: &str)
        -> Result<String, AdminError>
    {
        let resp: SimpleVersionId = self.post_json(
            &format!("/versions/{version_id}/rollback"),
            &serde_json::json!({ "reason": reason }),
        ).await?;
        Ok(resp.new_active_version_id)
    }

    pub async fn compute_diff(
        &self, from: &str, to: &str, gold_id: Option<&str>,
    ) -> Result<DiffSummary, AdminError> {
        let mut path = format!("/diff?from={from}&to={to}");
        if let Some(g) = gold_id { path.push_str(&format!("&gold_dataset_id={g}")); }
        self.get_json(&path).await
    }

    pub async fn propose_split(
        &self, catalog: &str, class_id: &str, max_splits: u32, sample_size: u32, mode: &str,
    ) -> Result<ToolResponse, AdminError> {
        self.post_json(
            "/tools/propose_split",
            &serde_json::json!({
                "catalog_id": catalog, "class_id": class_id,
                "max_splits": max_splits, "sample_size": sample_size,
                "execution_mode": mode,
            }),
        ).await
    }

    pub async fn propose_merge(
        &self, catalog: &str, class_ids: &[String], mode: &str,
    ) -> Result<ToolResponse, AdminError> {
        self.post_json(
            "/tools/suggest_merge",
            &serde_json::json!({
                "catalog_id": catalog, "class_ids": class_ids, "execution_mode": mode,
            }),
        ).await
    }

    pub async fn explain_misclassification(
        &self, catalog: &str, sentence_id: &str, actual: &str,
        expected: Option<&str>, mode: &str,
    ) -> Result<ExplainResponse, AdminError> {
        self.post_json(
            "/tools/explain",
            &serde_json::json!({
                "catalog_id": catalog, "sentence_id": sentence_id,
                "actual_class_id": actual, "expected_class_id": expected,
                "execution_mode": mode,
            }),
        ).await
    }

    pub async fn generate_synth(
        &self, catalog: &str, class_id: &str, n: u32, diversity: &str,
        include_negative: bool, mode: &str,
    ) -> Result<SynthResponse, AdminError> {
        self.post_json(
            "/tools/synth",
            &serde_json::json!({
                "catalog_id": catalog, "class_id": class_id, "n_examples": n,
                "diversity_hint": diversity, "include_negative": include_negative,
                "execution_mode": mode,
            }),
        ).await
    }

    pub async fn audit_bundle_vs_gold(
        &self, catalog: &str, bundle: &str, gold: &str,
        max_weak_spots: u32, with_llm: bool, mode: &str,
    ) -> Result<AuditResponse, AdminError> {
        self.post_json(
            "/tools/audit",
            &serde_json::json!({
                "catalog_id": catalog, "bundle_id": bundle, "gold_dataset_id": gold,
                "max_weak_spots": max_weak_spots,
                "include_llm_explanation": with_llm, "execution_mode": mode,
            }),
        ).await
    }

    pub async fn list_drift_events(
        &self, catalog: Option<&str>, min_severity: &str,
        only_unack: bool, limit: u32,
    ) -> Result<Vec<DriftEventSummary>, AdminError> {
        let mut path = format!("/drift?min_severity={min_severity}&limit={limit}");
        if only_unack { path.push_str("&only_unacknowledged=true"); }
        if let Some(c) = catalog { path.push_str(&format!("&catalog={c}")); }
        self.get_json(&path).await
    }

    pub async fn acknowledge_drift_event(&self, id: &str, note: Option<&str>) -> Result<(), AdminError> {
        let _: serde_json::Value = self.post_json(
            &format!("/drift/{id}/acknowledge"),
            &serde_json::json!({ "note": note }),
        ).await?;
        Ok(())
    }

    pub async fn drift_stats(&self, catalog: Option<&str>, days: u32) -> Result<DriftStats, AdminError> {
        let mut path = format!("/drift/stats?days={days}");
        if let Some(c) = catalog { path.push_str(&format!("&catalog={c}")); }
        self.get_json(&path).await
    }

    pub async fn cost_report(
        &self, from: DateTime<Utc>, to: DateTime<Utc>, group_by: &[String],
    ) -> Result<CostReport, AdminError> {
        self.post_json(
            "/cost/report",
            &serde_json::json!({
                "from": from.to_rfc3339(), "to": to.to_rfc3339(),
                "group_by": group_by,
            }),
        ).await
    }
}

// ─── DTOs ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProposalSummary {
    pub id: String,
    pub kind: String,
    pub status: String,
    /// D93 — added : was implicitly assumed in callers but never declared.
    #[serde(default)]
    pub tenant_id: String,
    pub catalog_id: String,
    pub proposed_by: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub review_count: u32,
    /// Renamed from the prior `cost_cents` to make the semantics explicit
    /// (this is the resolved spend, not the estimate). Old wire-format
    /// `cost_cents` still accepted via the `alias`.
    #[serde(default, alias = "cost_cents")]
    pub actual_cost_cents: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ProposalDetail {
    pub summary: ProposalSummary,
    pub rationale_markdown: String,
    pub diff_json: JsonValue,
    pub reviews: Vec<ReviewRecord>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRecord {
    pub id: String,
    pub reviewer_oidc_sub: String,
    pub reviewer_role: String,
    pub decision: String,
    pub rationale: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct PromoteResponse {
    pub candidate_version_id: String,
    pub canary_started_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct SimpleStatus { new_status: String }

#[derive(Debug, Deserialize)]
struct SimpleVersionId { new_active_version_id: String }

#[derive(Debug, Deserialize, Serialize)]
pub struct DiffSummary {
    pub from_semver: String,
    pub to_semver: String,
    pub class_changes: Vec<DiffClassChange>,
    pub estimated_impact: Option<DiffImpact>,
}

impl DiffSummary {
    pub fn class_changes(&self) -> &[DiffClassChange] { &self.class_changes }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiffClassChange {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub class_id: Option<String>,
    #[serde(default)]
    pub from: Option<JsonValue>,
    #[serde(default)]
    pub to: Option<JsonValue>,
}

impl DiffClassChange {
    pub fn kind_label(&self) -> &str { &self.kind }
    pub fn label_from(&self) -> String {
        self.class_id.clone()
            .or_else(|| self.from.as_ref().map(|v| v.to_string()))
            .unwrap_or_default()
    }
    pub fn label_to(&self) -> String {
        self.to.as_ref().map(|v| v.to_string()).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiffImpact {
    pub affected_examples_count: u64,
    pub estimated_accuracy_delta_pp: f32,
    pub re_label_required: u64,
}

#[derive(Debug, Deserialize)]
pub struct ToolResponse {
    pub proposal_id: String,
    #[serde(default)]
    pub llm_provider: String,
    #[serde(default)]
    pub cost_cents: u32,
    /// D93 — typed from `Vec<JsonValue>` to expose the fields the CLI
    /// already accesses (proposed_class_id, anchor_candidates).
    #[serde(default)]
    pub proposed_classes: Vec<ProposedSubclass>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProposedSubclass {
    pub proposed_class_id: String,
    #[serde(default)]
    pub anchor_candidates: Vec<String>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExplainResponse {
    pub proposal_id: String,
    #[serde(default)]
    pub root_cause_markdown: String,
    #[serde(default)]
    pub suggested_fixes: Vec<ExplainFix>,
}

#[derive(Debug, Deserialize)]
pub struct ExplainFix {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description_markdown: String,
}

impl ExplainFix { pub fn kind_label(&self) -> &str { &self.kind } }

#[derive(Debug, Deserialize)]
pub struct SynthResponse {
    pub proposal_id: String,
    #[serde(default)]
    pub examples: Vec<JsonValue>,
    #[serde(default)]
    pub cost_cents: u32,
    #[serde(default)]
    pub llm_provider: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditResponse {
    pub scorecard_id: String,
    pub accuracy: f32,
    pub macro_f1: f32,
    pub weighted_f1: f32,
    #[serde(default)]
    pub weak_spots: Vec<AuditWeakSpot>,
    #[serde(default)]
    pub llm_explanation: Option<String>,
    #[serde(default)]
    pub cost_cents: u32,
}

#[derive(Debug, Deserialize)]
pub struct AuditWeakSpot {
    pub class_id: String,
    pub description: String,
    pub severity: f32,
}

#[derive(Debug, Deserialize)]
pub struct DriftEventSummary {
    pub id: String,
    pub class_id: String,
    pub kl_divergence: f64,
    pub severity: String,
    pub detected_at: DateTime<Utc>,
    #[serde(default)]
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct DriftStats {
    pub total: u64, pub high: u64, pub medium: u64, pub low: u64, pub unack: u64,
}

#[derive(Debug, Deserialize)]
pub struct CostReport {
    pub total_cost_cents: u64,
    pub buckets: Vec<CostBucket>,
}

#[derive(Debug, Deserialize)]
pub struct CostBucket {
    pub labels: std::collections::BTreeMap<String, String>,
    pub call_count: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_cents: u64,
}
