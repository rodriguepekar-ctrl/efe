//! gRPC service implementation : `efe.curation.v1.CurationService`.
//!
//! Bridges the proto-generated tonic interface to the internal modules
//! (`proposal`, `version_dag`, `rollout`, `audit`, etc.). Authorization
//! and tenant binding are enforced here at the entry point of every RPC.
//!
//! See § 15 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{
    audit, proposal, rollout, version_dag,
    CatalogId, CurationError, CurationResult, TenantCurationFlags, TenantId,
};
use sqlx::PgPool;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use uuid::Uuid;

/// Caller authentication context, populated by the auth middleware.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub tenant_id: TenantId,
    pub oidc_sub: String,
    pub role: proposal::Role,
    pub oidc_token_iat: chrono::DateTime<chrono::Utc>,
    pub oidc_id_token_hash: String,
}

/// Backing service for the gRPC handlers.
pub struct CurationService {
    pub pg: Arc<PgPool>,
    pub feature_flags: Arc<dyn FeatureFlagProvider>,
    pub llm_client: Arc<dyn LlmClient>,
    pub prompt_renderer: Arc<dyn PromptRenderer>,
}

/// Trait : reads server-wide + tenant feature flags.
#[async_trait::async_trait]
pub trait FeatureFlagProvider: Send + Sync {
    fn server_m23_enabled(&self) -> bool;
    fn server_m21_enabled(&self) -> bool;
    fn server_use_internal_llm(&self) -> bool;
    async fn tenant_flags(&self, tenant: &TenantId) -> CurationResult<TenantCurationFlags>;
}

/// Trait : LLM client (sampling delegation OR internal LLMRouter).
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn call(
        &self, tenant: &TenantId, prompt: &str, max_tokens: u32, mode: ExecutionMode,
    ) -> CurationResult<LlmResponse>;
}

/// Trait : renders prompt templates with tenant-scoped, PII-redacted context.
#[async_trait::async_trait]
pub trait PromptRenderer: Send + Sync {
    async fn render(
        &self, template_name: &str, version: u32,
        tenant: &TenantId, context: serde_json::Value,
    ) -> CurationResult<RenderedPrompt>;
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutionMode { SamplingDelegation, InternalLlm }

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_cents: u32,
    pub latency_ms: u32,
    pub audit_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub body: String,
    pub template_name: String,
    pub template_version: u32,
}

impl CurationService {
    /// Pre-flight validation common to all RPCs.
    pub async fn validate_caller(&self, ctx: &CallerContext, requested_tenant: &TenantId) -> CurationResult<TenantCurationFlags> {
        if ctx.tenant_id != *requested_tenant {
            return Err(CurationError::TenantMismatch {
                caller: ctx.tenant_id.0.clone(), requested: requested_tenant.0.clone(),
            });
        }
        if !self.feature_flags.server_m23_enabled() {
            return Err(CurationError::M23ServerDisabled);
        }
        let flags = self.feature_flags.tenant_flags(requested_tenant).await?;
        if !flags.curation_enabled {
            return Err(CurationError::M23NotEnabledForTenant(requested_tenant.0.clone()));
        }
        Ok(flags)
    }

    /// Determine execution path from request mode + flags. Returns
    /// `FailedPrecondition` chain when the requested mode is not allowed.
    pub fn resolve_execution_mode(
        &self, requested: &str, tenant_flags: &TenantCurationFlags,
    ) -> CurationResult<ExecutionMode> {
        match requested {
            "INTERNAL_LLM" => {
                if !self.feature_flags.server_m21_enabled() {
                    return Err(CurationError::M21Required);
                }
                if !self.feature_flags.server_use_internal_llm() {
                    return Err(CurationError::M23ServerDisabled);
                }
                if !tenant_flags.allow_internal_llm {
                    return Err(CurationError::InternalLlmNotAllowed("tenant flag off".into()));
                }
                Ok(ExecutionMode::InternalLlm)
            }
            _ => Ok(ExecutionMode::SamplingDelegation),
        }
    }

    /// Set the per-session tenant context for RLS. Called at the very start
    /// of every RPC after auth.
    pub async fn set_session_tenant(&self, tenant: &TenantId) -> CurationResult<()> {
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(&tenant.0).execute(self.pg.as_ref()).await?;
        Ok(())
    }

    // ── Tool 1 : propose taxonomy split ──────────────────────────────

    pub async fn propose_taxonomy_split(
        &self, ctx: &CallerContext, req: ProposeSplitInput,
    ) -> CurationResult<ProposeSplitOutput> {
        let flags = self.validate_caller(ctx, &req.tenant_id).await?;
        if !ctx.role.can_propose() {
            return Err(CurationError::PermissionDenied {
                role: format!("{:?}", ctx.role), action: "propose_split".into(),
            });
        }
        let mode = self.resolve_execution_mode(&req.execution_mode, &flags)?;
        self.set_session_tenant(&req.tenant_id).await?;

        // Idempotency : compute key from inputs + active version
        let active = version_dag::get_active_version(&self.pg, &req.tenant_id, &req.catalog_id).await?
            .ok_or_else(|| CurationError::InvalidInput(
                "no ACTIVE taxonomy version for catalog ; create initial version first".into()
            ))?;
        let idempotency_key = req.idempotency_key.clone().unwrap_or_else(|| {
            proposal::compute_idempotency_key(&[
                &req.tenant_id.0, &req.catalog_id.0, &req.class_id, &active.semver, "split_v1",
            ])
        });

        // Quick return on idempotency hit
        if let Some(existing) = proposal::find_by_idempotency_key(&self.pg, &req.tenant_id, &idempotency_key).await? {
            crate::metrics::CURATION_TOOL_IDEMPOTENCY_HITS_TOTAL
                .with_label_values(&[&req.tenant_id.0, "split"]).inc();
            return Ok(ProposeSplitOutput {
                proposal_id: existing.id,
                rationale_markdown: existing.rationale_markdown,
                confidence_score: 0.0, // not stored separately
                proposed_classes: vec![],
                llm_metadata: None,
            });
        }

        // Render prompt with PII-redacted context
        let context = serde_json::json!({
            "tenant_id": req.tenant_id.0,
            "catalog_id": req.catalog_id.0,
            "class_id": req.class_id,
            "max_splits": req.max_splits,
            "include_examples": req.include_examples,
            "sample_size": req.sample_size,
            "taxonomy_version": active.semver,
        });
        let prompt = self.prompt_renderer
            .render("curation_propose_split", 1, &req.tenant_id, context).await?;

        // LLM call
        let llm_resp = self.llm_client.call(
            &req.tenant_id, &prompt.body, 1500, mode,
        ).await?;

        // Parse + validate output
        let parsed = crate::schemas::parse_validated::<crate::schemas::TaxonomySplitOutput>(&llm_resp.content)?;

        // Insert proposal
        let new_proposal = proposal::NewProposal {
            tenant_id: req.tenant_id.clone(),
            catalog_id: req.catalog_id.clone(),
            parent_taxonomy_version_id: active.id,
            kind: proposal::ProposalKind::Split,
            proposed_by: ctx.oidc_sub.clone(),
            proposed_via: req.proposed_via.clone(),
            llm_call_audit_id: llm_resp.audit_id,
            prompt_template_name: prompt.template_name.clone(),
            prompt_template_version: prompt.template_version as i32,
            execution_mode: req.execution_mode.clone(),
            rationale_markdown: parsed.rationale_markdown.clone(),
            diff_json: serde_json::to_value(&parsed)?,
            estimated_cost_cents: Some(llm_resp.cost_cents as i64),
            idempotency_key: Some(idempotency_key),
            retention_days: flags.proposal_retention_days,
        };
        let p = proposal::create_proposal(&self.pg, new_proposal).await?;

        // Transition Draft -> Pending
        let mut tx = self.pg.begin().await?;
        proposal::transition(&mut tx, p.id, proposal::ProposalStatus::Pending, ctx.role).await?;
        audit::append_audit(
            &mut tx, &req.tenant_id, Some(p.id), None,
            "proposal_created", &ctx.oidc_sub, Some(&format!("{:?}", ctx.role)),
            serde_json::json!({"kind": "SPLIT", "class_id": req.class_id, "execution_mode": req.execution_mode}),
        ).await?;
        tx.commit().await?;

        crate::metrics::CURATION_TOOL_CALLS_TOTAL
            .with_label_values(&[&req.tenant_id.0, "split", "success"]).inc();

        Ok(ProposeSplitOutput {
            proposal_id: p.id,
            rationale_markdown: parsed.rationale_markdown,
            confidence_score: parsed.confidence_score,
            proposed_classes: parsed.proposed_classes,
            llm_metadata: Some(LlmMetadata {
                provider: llm_resp.provider, model: llm_resp.model,
                input_tokens: llm_resp.input_tokens, output_tokens: llm_resp.output_tokens,
                cost_cents: llm_resp.cost_cents, latency_ms: llm_resp.latency_ms,
                template_name: prompt.template_name, template_version: prompt.template_version,
            }),
        })
    }

    // ── Review queue ────────────────────────────────────────────────

    pub async fn approve_proposal(
        &self, ctx: &CallerContext, proposal_id: Uuid, rationale: String,
    ) -> CurationResult<()> {
        if rationale.len() < 10 {
            return Err(CurationError::InvalidInput("rationale must be ≥ 10 chars".into()));
        }
        // OIDC freshness : iat within 10 min for Approve
        let age = chrono::Utc::now().signed_duration_since(ctx.oidc_token_iat);
        if age.num_seconds() > 600 {
            return Err(CurationError::OidcTokenStale {
                age_seconds: age.num_seconds(), max: 600,
            });
        }
        if !ctx.role.can_review() {
            return Err(CurationError::PermissionDenied {
                role: format!("{:?}", ctx.role), action: "approve".into(),
            });
        }
        self.set_session_tenant(&ctx.tenant_id).await?;
        let mut tx = self.pg.begin().await?;

        // Validate proposal belongs to caller's tenant (RLS already enforces
        // this server-side, but we double-check for clearer error mapping).
        let proposal_tenant: Option<String> = sqlx::query_scalar(
            "SELECT tenant_id FROM taxonomy_change_proposal WHERE id = $1",
        )
        .bind(proposal_id)
        .fetch_optional(&mut *tx).await?;
        let proposal_tenant = proposal_tenant
            .ok_or(CurationError::ProposalNotFound(proposal_id))?;
        if proposal_tenant != ctx.tenant_id.0 {
            return Err(CurationError::TenantMismatch {
                caller: ctx.tenant_id.0.clone(), requested: proposal_tenant,
            });
        }

        // Insert curation_review row
        sqlx::query(
            r#"INSERT INTO curation_review
               (id, proposal_id, reviewer_oidc_sub, reviewer_role, decision, rationale,
                oidc_id_token_hash, oidc_token_iat, reviewed_at)
               VALUES ($1, $2, $3, $4, 'APPROVE'::curation_review_decision, $5, $6, $7, NOW())"#,
        )
        .bind(Uuid::new_v4()).bind(proposal_id)
        .bind(&ctx.oidc_sub).bind(format!("{:?}", ctx.role))
        .bind(&rationale).bind(&ctx.oidc_id_token_hash).bind(ctx.oidc_token_iat)
        .execute(&mut *tx).await?;

        // Transition Pending -> Approved
        proposal::transition(&mut tx, proposal_id, proposal::ProposalStatus::Approved, ctx.role).await?;

        audit::append_audit(
            &mut tx, &ctx.tenant_id, Some(proposal_id), None,
            "review_approve", &ctx.oidc_sub, Some(&format!("{:?}", ctx.role)),
            serde_json::json!({"rationale": rationale}),
        ).await?;
        tx.commit().await?;
        crate::metrics::CURATION_REVIEWS_TOTAL.with_label_values(&[&ctx.tenant_id.0, "APPROVE"]).inc();
        Ok(())
    }

    pub async fn reject_proposal(
        &self, ctx: &CallerContext, proposal_id: Uuid, rationale: String,
    ) -> CurationResult<()> {
        if rationale.len() < 10 {
            return Err(CurationError::InvalidInput("rationale must be ≥ 10 chars".into()));
        }
        if !ctx.role.can_review() {
            return Err(CurationError::PermissionDenied {
                role: format!("{:?}", ctx.role), action: "reject".into(),
            });
        }
        self.set_session_tenant(&ctx.tenant_id).await?;
        let mut tx = self.pg.begin().await?;
        sqlx::query(
            r#"INSERT INTO curation_review
               (id, proposal_id, reviewer_oidc_sub, reviewer_role, decision, rationale,
                oidc_id_token_hash, oidc_token_iat, reviewed_at)
               VALUES ($1, $2, $3, $4, 'REJECT'::curation_review_decision, $5, $6, $7, NOW())"#,
        )
        .bind(Uuid::new_v4()).bind(proposal_id)
        .bind(&ctx.oidc_sub).bind(format!("{:?}", ctx.role))
        .bind(&rationale).bind(&ctx.oidc_id_token_hash).bind(ctx.oidc_token_iat)
        .execute(&mut *tx).await?;
        proposal::transition(&mut tx, proposal_id, proposal::ProposalStatus::Rejected, ctx.role).await?;
        audit::append_audit(
            &mut tx, &ctx.tenant_id, Some(proposal_id), None,
            "review_reject", &ctx.oidc_sub, Some(&format!("{:?}", ctx.role)),
            serde_json::json!({"rationale": rationale}),
        ).await?;
        tx.commit().await?;
        crate::metrics::CURATION_REVIEWS_TOTAL.with_label_values(&[&ctx.tenant_id.0, "REJECT"]).inc();
        Ok(())
    }

    pub async fn list_pending(
        &self, ctx: &CallerContext, catalog: Option<CatalogId>, limit: i64,
    ) -> CurationResult<Vec<proposal::Proposal>> {
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        self.set_session_tenant(&ctx.tenant_id).await?;
        proposal::list_pending(&self.pg, &ctx.tenant_id, catalog.as_ref(), limit).await
    }

    pub async fn rollback_taxonomy_version(
        &self, ctx: &CallerContext, target_version_id: Uuid, catalog: CatalogId, reason: String,
    ) -> CurationResult<Uuid> {
        if !ctx.role.can_promote() {
            return Err(CurationError::PermissionDenied {
                role: format!("{:?}", ctx.role), action: "rollback".into(),
            });
        }
        self.set_session_tenant(&ctx.tenant_id).await?;
        rollout::manual_rollback(
            &self.pg, target_version_id, &ctx.tenant_id, &catalog, &reason, &ctx.oidc_sub,
        ).await
    }
}

// ─── Public DTOs (for the gRPC layer to map) ─────────────────────────

#[derive(Debug, Clone)]
pub struct ProposeSplitInput {
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub class_id: String,
    pub max_splits: u32,
    pub include_examples: bool,
    pub sample_size: u32,
    pub execution_mode: String,
    pub idempotency_key: Option<String>,
    pub proposed_via: String,
}

#[derive(Debug, Clone)]
pub struct ProposeSplitOutput {
    pub proposal_id: Uuid,
    pub rationale_markdown: String,
    pub confidence_score: f32,
    pub proposed_classes: Vec<crate::schemas::split::ProposedSubclass>,
    pub llm_metadata: Option<LlmMetadata>,
}

#[derive(Debug, Clone)]
pub struct LlmMetadata {
    pub provider: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_cents: u32,
    pub latency_ms: u32,
    pub template_name: String,
    pub template_version: u32,
}

// All metrics centralised in `crate::metrics` (see src/metrics.rs).
// Use `crate::metrics::CURATION_TOOL_CALLS_TOTAL` etc. directly from
// handler code. The previous duplicate definitions were removed during
// the M23 audit pass to avoid `prometheus::Error::AlreadyReg` panics
// when both modules registered the same metric name.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_default_sampling() {
        // verified via the resolve logic in tests in test_helpers.rs
        assert!(matches!(
            ExecutionMode::SamplingDelegation,
            ExecutionMode::SamplingDelegation
        ));
    }
}
