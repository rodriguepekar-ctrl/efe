//! gRPC service implementation : `efe.active_learning.v1.ActiveLearningService`.
//!
//! Bridges proto-generated tonic traits to the internal modules. Auth +
//! tenant binding enforced at every entry point. Authorization matrix
//! per RPC documented in § 20.2 of the architecture doc.

use crate::{
    auto_confirm, gdpr, queue, retune, review,
    ActiveLearningError, ActiveLearningResult, CatalogId, TenantActiveLearningFlags, TenantId,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Caller authentication context, populated by the auth middleware.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub tenant_id: TenantId,
    pub oidc_sub: String,
    pub role: Role,
    pub oidc_token_iat: DateTime<Utc>,
    pub oidc_id_token_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Viewer,
    CurationProposer,
    CurationReviewer,
    CurationPromoter,
    TaxonomyAdmin,
    TenantAdmin,
}

impl Role {
    pub fn can_review(self) -> bool {
        matches!(self, Self::CurationReviewer | Self::TaxonomyAdmin | Self::TenantAdmin)
    }
    pub fn can_promote(self) -> bool {
        matches!(self, Self::CurationPromoter | Self::TaxonomyAdmin | Self::TenantAdmin)
    }
    pub fn is_tenant_admin(self) -> bool {
        matches!(self, Self::TenantAdmin)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::CurationProposer => "curation_proposer",
            Self::CurationReviewer => "curation_reviewer",
            Self::CurationPromoter => "curation_promoter",
            Self::TaxonomyAdmin => "taxonomy_admin",
            Self::TenantAdmin => "tenant_admin",
        }
    }
}

/// Service handler wrapper.
pub struct ActiveLearningService {
    pub pg: Arc<PgPool>,
    pub feature_flags: Arc<dyn FeatureFlagProvider>,
}

#[async_trait::async_trait]
pub trait FeatureFlagProvider: Send + Sync {
    fn server_m24_enabled(&self) -> bool;
    fn server_m21_enabled(&self) -> bool;
    fn server_m23_enabled(&self) -> bool;
    async fn tenant_flags(&self, tenant: &TenantId) -> ActiveLearningResult<TenantActiveLearningFlags>;
}

impl ActiveLearningService {
    pub async fn validate_caller(
        &self, ctx: &CallerContext, requested_tenant: &TenantId,
    ) -> ActiveLearningResult<TenantActiveLearningFlags> {
        if ctx.tenant_id != *requested_tenant {
            return Err(ActiveLearningError::TenantMismatch {
                caller: ctx.tenant_id.0.clone(), requested: requested_tenant.0.clone(),
            });
        }
        if !self.feature_flags.server_m24_enabled() {
            return Err(ActiveLearningError::M24ServerDisabled);
        }
        let flags = self.feature_flags.tenant_flags(requested_tenant).await?;
        if !flags.active_learning_enabled {
            return Err(ActiveLearningError::M24NotEnabledForTenant(
                requested_tenant.0.clone()
            ));
        }
        Ok(flags)
    }

    pub async fn set_session_tenant(&self, tenant: &TenantId) -> ActiveLearningResult<()> {
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(&tenant.0).execute(self.pg.as_ref()).await?;
        Ok(())
    }

    // ── List queue ──────────────────────────────────────────────────

    pub async fn list_queue(
        &self, ctx: &CallerContext, status_filter: Option<queue::QueueStatus>,
        limit: i64,
    ) -> ActiveLearningResult<Vec<queue::QueueRow>> {
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        self.set_session_tenant(&ctx.tenant_id).await?;

        let rows = if let Some(s) = status_filter {
            sqlx::query_as::<_, queue::QueueRow>(
                r#"SELECT id, tenant_id, catalog_id, content_hash, sentence_redacted,
                          proposed_class_id, observed_confidence, status,
                          source_llm_call_id, enqueued_at, labeled_at, confirmed_at,
                          failure_count, last_error
                   FROM gold_label_queue
                   WHERE tenant_id = $1 AND status = $2::gold_label_queue_status
                   ORDER BY enqueued_at DESC
                   LIMIT $3"#,
            )
            .bind(&ctx.tenant_id.0).bind(s.as_str()).bind(limit)
            .fetch_all(self.pg.as_ref()).await?
        } else {
            sqlx::query_as::<_, queue::QueueRow>(
                r#"SELECT id, tenant_id, catalog_id, content_hash, sentence_redacted,
                          proposed_class_id, observed_confidence, status,
                          source_llm_call_id, enqueued_at, labeled_at, confirmed_at,
                          failure_count, last_error
                   FROM gold_label_queue
                   WHERE tenant_id = $1
                   ORDER BY enqueued_at DESC
                   LIMIT $2"#,
            )
            .bind(&ctx.tenant_id.0).bind(limit)
            .fetch_all(self.pg.as_ref()).await?
        };
        Ok(rows)
    }

    // ── Confirm / Reject / Edit ─────────────────────────────────────

    pub async fn confirm_label(
        &self, ctx: &CallerContext, proposal_id: Uuid, rationale: String,
    ) -> ActiveLearningResult<Uuid> {
        if !ctx.role.can_review() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(), action: "confirm_label".into(),
            });
        }
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        review::submit_review(self.pg.as_ref(), &ctx.tenant_id, review::ReviewRequest {
            proposal_id,
            reviewer_oidc_sub: ctx.oidc_sub.clone(),
            reviewer_role: ctx.role.as_str().into(),
            decision: review::ReviewDecision::Confirm,
            final_class_id: None,
            rationale,
            quality_tag: None,
            oidc_id_token_hash: ctx.oidc_id_token_hash.clone(),
            oidc_token_iat: ctx.oidc_token_iat,
        }).await
    }

    pub async fn edit_and_confirm_label(
        &self, ctx: &CallerContext, proposal_id: Uuid,
        final_class_id: String, rationale: String,
    ) -> ActiveLearningResult<Uuid> {
        if !ctx.role.can_review() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(), action: "edit_and_confirm".into(),
            });
        }
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        review::submit_review(self.pg.as_ref(), &ctx.tenant_id, review::ReviewRequest {
            proposal_id,
            reviewer_oidc_sub: ctx.oidc_sub.clone(),
            reviewer_role: ctx.role.as_str().into(),
            decision: review::ReviewDecision::EditAndConfirm,
            final_class_id: Some(final_class_id),
            rationale,
            quality_tag: Some("edited".into()),
            oidc_id_token_hash: ctx.oidc_id_token_hash.clone(),
            oidc_token_iat: ctx.oidc_token_iat,
        }).await
    }

    pub async fn reject_label(
        &self, ctx: &CallerContext, proposal_id: Uuid, rationale: String,
    ) -> ActiveLearningResult<Uuid> {
        if !ctx.role.can_review() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(), action: "reject_label".into(),
            });
        }
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        review::submit_review(self.pg.as_ref(), &ctx.tenant_id, review::ReviewRequest {
            proposal_id,
            reviewer_oidc_sub: ctx.oidc_sub.clone(),
            reviewer_role: ctx.role.as_str().into(),
            decision: review::ReviewDecision::Reject,
            final_class_id: None,
            rationale,
            quality_tag: None,
            oidc_id_token_hash: ctx.oidc_id_token_hash.clone(),
            oidc_token_iat: ctx.oidc_token_iat,
        }).await
    }

    // ── Trigger retune ──────────────────────────────────────────────

    /// Trigger a retune. Inserts a `bundle_retune_run` row in TUNING
    /// status and returns the identifier. The orchestrator worker
    /// (`retune::scheduler_loop`) picks up TUNING rows on the next tick
    /// and runs the centroid recompute + eval pipeline asynchronously —
    /// this RPC only enqueues the work.
    pub async fn trigger_retune(
        &self, ctx: &CallerContext, catalog: CatalogId, force_below_threshold: bool,
    ) -> ActiveLearningResult<TriggerRetuneOutput> {
        if !ctx.role.can_promote() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(), action: "trigger_retune".into(),
            });
        }
        if force_below_threshold && !ctx.role.is_tenant_admin() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(),
                action: "force_below_threshold (tenant_admin only)".into(),
            });
        }
        let flags = self.validate_caller(ctx, &ctx.tenant_id).await?;
        self.set_session_tenant(&ctx.tenant_id).await?;

        // Threshold check (unless admin overrode).
        if !force_below_threshold {
            let new_count = retune::count_new_examples_since_last_retune(
                self.pg.as_ref(), &ctx.tenant_id, &catalog,
            ).await?;
            if (new_count as u32) < flags.gold_label_threshold {
                return Err(ActiveLearningError::InsufficientGoldExamples {
                    actual: new_count, required: flags.gold_label_threshold as u64,
                });
            }
        }

        // Look up parent (currently ACTIVE) taxonomy_version + latest gold_dataset.
        let parent: Option<(uuid::Uuid, String)> = sqlx::query_as(
            r#"SELECT id, semver
               FROM taxonomy_version
               WHERE tenant_id = $1 AND catalog_id = $2 AND status = 'ACTIVE'
               LIMIT 1"#,
        )
        .bind(&ctx.tenant_id.0).bind(&catalog.0)
        .fetch_optional(self.pg.as_ref()).await?;
        let (parent_id, parent_semver) = parent.ok_or_else(|| {
            ActiveLearningError::ActiveBundleNotFound {
                tenant: ctx.tenant_id.0.clone(), catalog: catalog.0.clone(),
            }
        })?;
        let new_semver = retune::bump_patch(&parent_semver)?;

        let dataset_id: uuid::Uuid = sqlx::query_scalar(
            r#"SELECT id FROM gold_dataset
               WHERE tenant_id = $1 AND catalog_id = $2
               ORDER BY created_at DESC
               LIMIT 1"#,
        )
        .bind(&ctx.tenant_id.0).bind(&catalog.0)
        .fetch_optional(self.pg.as_ref()).await?
        .ok_or_else(|| ActiveLearningError::InvalidInput(format!(
            "no gold_dataset for tenant={} catalog={}", ctx.tenant_id.0, catalog.0,
        )))?;

        let trigger = retune::RetuneTrigger {
            kind: retune::TriggerKind::OnDemand,
            actor: Some(ctx.oidc_sub.clone()),
        };
        let run_id = retune::insert_retune_run(
            self.pg.as_ref(), &ctx.tenant_id, &catalog,
            parent_id, &new_semver, dataset_id,
            /* training/holdout counts populated by worker */ 0, 0,
            &trigger,
        ).await?;

        Ok(TriggerRetuneOutput { run_id, started_at: chrono::Utc::now() })
    }

    // ── GDPR Art. 17 ────────────────────────────────────────────────

    pub async fn submit_gdpr_erasure(
        &self, ctx: &CallerContext, req: gdpr::ErasureRequest,
    ) -> ActiveLearningResult<gdpr::RebuildRequestSummary> {
        if !ctx.role.is_tenant_admin() {
            return Err(ActiveLearningError::PermissionDenied {
                role: ctx.role.as_str().into(), action: "gdpr_erasure".into(),
            });
        }
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        gdpr::submit_erasure(self.pg.as_ref(), req).await
    }

    pub async fn list_gdpr_pending(
        &self, ctx: &CallerContext,
    ) -> ActiveLearningResult<Vec<gdpr::RebuildRequestSummary>> {
        self.validate_caller(ctx, &ctx.tenant_id).await?;
        self.set_session_tenant(&ctx.tenant_id).await?;
        gdpr::list_pending(self.pg.as_ref(), &ctx.tenant_id).await
    }

    // ── Auto-confirm gate evaluation (preview, no apply) ────────────

    pub async fn check_auto_confirm(
        &self, ctx: &CallerContext, proposal_id: Uuid,
    ) -> ActiveLearningResult<auto_confirm::AutoConfirmDecision> {
        let flags = self.validate_caller(ctx, &ctx.tenant_id).await?;
        self.set_session_tenant(&ctx.tenant_id).await?;
        auto_confirm::check_proposal(self.pg.as_ref(), &ctx.tenant_id, proposal_id, &flags).await
    }
}

/// Output DTO for `trigger_retune` — exposed via gRPC `TriggerBundleRetuneResponse`.
#[derive(Debug, Clone)]
pub struct TriggerRetuneOutput {
    pub run_id: Uuid,
    pub started_at: DateTime<Utc>,
}
