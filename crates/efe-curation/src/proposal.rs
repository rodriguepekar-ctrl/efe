//! Proposal lifecycle manager.
//!
//! A `taxonomy_change_proposal` represents a suggested change to a taxonomy,
//! produced by an LLM-assisted curation tool. Proposals follow a strict
//! state machine ; transitions are validated by `validate_transition` and
//! enforced by Postgres triggers + this module.
//!
//! See § 10 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{CurationError, CurationResult, TenantId, CatalogId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "proposal_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProposalStatus {
    Draft, Pending, Approved, Rejected, Promoted, RolledBack, Expired,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "DRAFT", Self::Pending => "PENDING",
            Self::Approved => "APPROVED", Self::Rejected => "REJECTED",
            Self::Promoted => "PROMOTED", Self::RolledBack => "ROLLED_BACK",
            Self::Expired => "EXPIRED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "proposal_kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProposalKind {
    Split, Merge, Explain, GenerateSynth, AuditVsGold,
}

impl ProposalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Split => "SPLIT", Self::Merge => "MERGE",
            Self::Explain => "EXPLAIN", Self::GenerateSynth => "GENERATE_SYNTH",
            Self::AuditVsGold => "AUDIT_VS_GOLD",
        }
    }
}

/// A reviewer's role for transition authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Viewer, CurationProposer, CurationReviewer,
    CurationPromoter, TaxonomyAdmin, TenantAdmin,
}

impl Role {
    pub fn can_review(self) -> bool {
        matches!(self, Self::CurationReviewer | Self::TaxonomyAdmin | Self::TenantAdmin)
    }
    pub fn can_promote(self) -> bool {
        matches!(self, Self::CurationPromoter | Self::TaxonomyAdmin | Self::TenantAdmin)
    }
    pub fn can_propose(self) -> bool {
        matches!(self, Self::CurationProposer | Self::CurationReviewer | Self::CurationPromoter | Self::TaxonomyAdmin | Self::TenantAdmin)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub parent_taxonomy_version_id: Uuid,
    pub candidate_taxonomy_version_id: Option<Uuid>,
    pub kind: ProposalKind,
    pub status: ProposalStatus,
    pub proposed_by: String,
    pub proposed_via: String,
    pub llm_call_audit_id: Option<Uuid>,
    pub prompt_template_name: String,
    pub prompt_template_version: i32,
    pub execution_mode: String,
    pub rationale_markdown: String,
    pub diff_json: serde_json::Value,
    pub estimated_cost_cents: Option<i64>,
    pub actual_cost_cents: Option<i64>,
    pub idempotency_key: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Validate a status transition is allowed for the given actor role.
///
/// Returns `Ok(())` if allowed, `CurationError::InvalidTransition` otherwise.
pub fn validate_transition(
    from: ProposalStatus, to: ProposalStatus, actor_role: Role,
) -> CurationResult<()> {
    use ProposalStatus::*;
    let allowed = match (from, to) {
        (Draft, Pending) | (Draft, Rejected) => true,
        (Pending, Approved) | (Pending, Rejected) => actor_role.can_review(),
        (Approved, Promoted) => actor_role.can_promote(),
        (Promoted, RolledBack) => actor_role.can_promote(),
        (Pending, Expired) | (Approved, Expired) => true, // system-driven
        _ => false,
    };
    if !allowed {
        return Err(CurationError::InvalidTransition {
            from: from.as_str().into(), to: to.as_str().into(),
        });
    }
    Ok(())
}

/// Compute a deterministic idempotency key from the input vector.
///
/// Used by tools to deduplicate identical calls within retention window.
pub fn compute_idempotency_key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts { h.update(p.as_bytes()); h.update(b"|"); }
    hex::encode(h.finalize())
}

/// Insert a new proposal in DRAFT status. Performs idempotency lookup first.
pub async fn create_proposal(
    pg: &PgPool, new: NewProposal,
) -> CurationResult<Proposal> {
    if let Some(ref key) = new.idempotency_key {
        if let Some(existing) = find_by_idempotency_key(pg, &new.tenant_id, key).await? {
            tracing::debug!(?existing.id, "idempotency key matched ; returning existing proposal");
            return Ok(existing);
        }
    }
    let id = Uuid::new_v4();
    let now = Utc::now();
    let expires_at = now + Duration::days(new.retention_days as i64);
    sqlx::query(
        r#"
        INSERT INTO taxonomy_change_proposal (
            id, tenant_id, catalog_id, parent_taxonomy_version_id,
            kind, status, proposed_by, proposed_via, llm_call_audit_id,
            prompt_template_name, prompt_template_version, execution_mode,
            rationale_markdown, diff_json, estimated_cost_cents,
            idempotency_key, expires_at, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4,
            $5::proposal_kind, 'DRAFT'::proposal_status, $6, $7, $8,
            $9, $10, $11,
            $12, $13, $14,
            $15, $16, $17, $17
        )
        "#,
    )
    .bind(id).bind(&new.tenant_id.0).bind(&new.catalog_id.0)
    .bind(new.parent_taxonomy_version_id)
    .bind(new.kind.as_str()).bind(&new.proposed_by).bind(&new.proposed_via)
    .bind(new.llm_call_audit_id)
    .bind(&new.prompt_template_name).bind(new.prompt_template_version)
    .bind(&new.execution_mode)
    .bind(&new.rationale_markdown).bind(&new.diff_json)
    .bind(new.estimated_cost_cents)
    .bind(&new.idempotency_key).bind(expires_at)
    .bind(now)
    .execute(pg).await?;

    Ok(Proposal {
        id, tenant_id: new.tenant_id, catalog_id: new.catalog_id,
        parent_taxonomy_version_id: new.parent_taxonomy_version_id,
        candidate_taxonomy_version_id: None,
        kind: new.kind, status: ProposalStatus::Draft,
        proposed_by: new.proposed_by, proposed_via: new.proposed_via,
        llm_call_audit_id: new.llm_call_audit_id,
        prompt_template_name: new.prompt_template_name,
        prompt_template_version: new.prompt_template_version,
        execution_mode: new.execution_mode,
        rationale_markdown: new.rationale_markdown, diff_json: new.diff_json,
        estimated_cost_cents: new.estimated_cost_cents, actual_cost_cents: None,
        idempotency_key: new.idempotency_key, expires_at, created_at: now, updated_at: now,
    })
}

#[derive(Debug, Clone)]
pub struct NewProposal {
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub parent_taxonomy_version_id: Uuid,
    pub kind: ProposalKind,
    pub proposed_by: String,
    pub proposed_via: String,
    pub llm_call_audit_id: Option<Uuid>,
    pub prompt_template_name: String,
    pub prompt_template_version: i32,
    pub execution_mode: String,
    pub rationale_markdown: String,
    pub diff_json: serde_json::Value,
    pub estimated_cost_cents: Option<i64>,
    pub idempotency_key: Option<String>,
    pub retention_days: u32,
}

/// Look up a proposal by idempotency key. Returns `None` if not found or expired.
pub async fn find_by_idempotency_key(
    pg: &PgPool, tenant: &TenantId, key: &str,
) -> CurationResult<Option<Proposal>> {
    let row = sqlx::query_as::<_, ProposalRow>(
        r#"SELECT * FROM taxonomy_change_proposal
           WHERE tenant_id = $1 AND idempotency_key = $2 AND expires_at > NOW()
           LIMIT 1"#,
    )
    .bind(&tenant.0).bind(key)
    .fetch_optional(pg).await?;
    Ok(row.map(Into::into))
}

/// Transition a proposal to a new status, validated by role + state machine.
///
/// Uses runtime queries (not the `sqlx::query!` macro) to keep the crate
/// compilable without a live `DATABASE_URL` at build time. The macro path
/// would force every contributor to run `cargo sqlx prepare` after each
/// migration churn, which is friction that does not buy type-checking we
/// don't already get from the explicit `query_scalar::<_, ProposalStatus>`.
pub async fn transition(
    tx: &mut Transaction<'_, Postgres>, proposal_id: Uuid, to: ProposalStatus, actor_role: Role,
) -> CurationResult<()> {
    let current: Option<ProposalStatus> = sqlx::query_scalar(
        r#"SELECT status FROM taxonomy_change_proposal WHERE id = $1"#,
    )
    .bind(proposal_id)
    .fetch_optional(&mut **tx).await?;

    let from = current.ok_or(CurationError::ProposalNotFound(proposal_id))?;
    validate_transition(from, to, actor_role)?;

    sqlx::query(
        r#"UPDATE taxonomy_change_proposal
           SET status = $1::proposal_status, updated_at = NOW()
           WHERE id = $2"#,
    )
    .bind(to.as_str()).bind(proposal_id)
    .execute(&mut **tx).await?;
    Ok(())
}

/// Mark expired proposals — runs as scheduled job.
pub async fn expire_old_proposals(pg: &PgPool) -> CurationResult<u64> {
    let res = sqlx::query(
        r#"UPDATE taxonomy_change_proposal
           SET status = 'EXPIRED'::proposal_status, updated_at = NOW()
           WHERE status IN ('PENDING', 'APPROVED') AND expires_at < NOW()"#,
    )
    .execute(pg).await?;
    Ok(res.rows_affected())
}

/// List pending proposals for a tenant, ordered by priority + age.
pub async fn list_pending(
    pg: &PgPool, tenant: &TenantId, catalog: Option<&CatalogId>, limit: i64,
) -> CurationResult<Vec<Proposal>> {
    let rows = sqlx::query_as::<_, ProposalRow>(
        r#"SELECT * FROM taxonomy_change_proposal
           WHERE tenant_id = $1
             AND ($2::TEXT IS NULL OR catalog_id = $2)
             AND status IN ('PENDING', 'APPROVED')
           ORDER BY created_at ASC
           LIMIT $3"#,
    )
    .bind(&tenant.0)
    .bind(catalog.map(|c| c.0.as_str()))
    .bind(limit)
    .fetch_all(pg).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

// ─── DB row mapping ─────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct ProposalRow {
    id: Uuid,
    tenant_id: String,
    catalog_id: String,
    parent_taxonomy_version_id: Uuid,
    candidate_taxonomy_version_id: Option<Uuid>,
    kind: ProposalKind,
    status: ProposalStatus,
    proposed_by: String,
    proposed_via: String,
    llm_call_audit_id: Option<Uuid>,
    prompt_template_name: String,
    prompt_template_version: i32,
    execution_mode: String,
    rationale_markdown: String,
    diff_json: serde_json::Value,
    estimated_cost_cents: Option<i64>,
    actual_cost_cents: Option<i64>,
    idempotency_key: Option<String>,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ProposalRow> for Proposal {
    fn from(r: ProposalRow) -> Self {
        Self {
            id: r.id,
            tenant_id: TenantId(r.tenant_id),
            catalog_id: CatalogId(r.catalog_id),
            parent_taxonomy_version_id: r.parent_taxonomy_version_id,
            candidate_taxonomy_version_id: r.candidate_taxonomy_version_id,
            kind: r.kind, status: r.status,
            proposed_by: r.proposed_by, proposed_via: r.proposed_via,
            llm_call_audit_id: r.llm_call_audit_id,
            prompt_template_name: r.prompt_template_name,
            prompt_template_version: r.prompt_template_version,
            execution_mode: r.execution_mode,
            rationale_markdown: r.rationale_markdown,
            diff_json: r.diff_json,
            estimated_cost_cents: r.estimated_cost_cents,
            actual_cost_cents: r.actual_cost_cents,
            idempotency_key: r.idempotency_key,
            expires_at: r.expires_at, created_at: r.created_at, updated_at: r.updated_at,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_draft_to_pending_anyone() {
        // DRAFT -> PENDING is system-internal, allowed for any role.
        assert!(validate_transition(ProposalStatus::Draft, ProposalStatus::Pending, Role::Viewer).is_ok());
    }

    #[test]
    fn test_transition_pending_to_approved_requires_reviewer() {
        assert!(validate_transition(ProposalStatus::Pending, ProposalStatus::Approved, Role::Viewer).is_err());
        assert!(validate_transition(ProposalStatus::Pending, ProposalStatus::Approved, Role::CurationReviewer).is_ok());
        assert!(validate_transition(ProposalStatus::Pending, ProposalStatus::Approved, Role::TaxonomyAdmin).is_ok());
    }

    #[test]
    fn test_transition_approved_to_promoted_requires_promoter() {
        assert!(validate_transition(ProposalStatus::Approved, ProposalStatus::Promoted, Role::CurationReviewer).is_err());
        assert!(validate_transition(ProposalStatus::Approved, ProposalStatus::Promoted, Role::CurationPromoter).is_ok());
        assert!(validate_transition(ProposalStatus::Approved, ProposalStatus::Promoted, Role::TenantAdmin).is_ok());
    }

    #[test]
    fn test_transition_disallowed_skip() {
        // Cannot skip DRAFT -> PROMOTED directly.
        assert!(validate_transition(ProposalStatus::Draft, ProposalStatus::Promoted, Role::TenantAdmin).is_err());
    }

    #[test]
    fn test_transition_rejected_terminal() {
        // REJECTED is terminal — cannot leave it.
        assert!(validate_transition(ProposalStatus::Rejected, ProposalStatus::Pending, Role::TenantAdmin).is_err());
        assert!(validate_transition(ProposalStatus::Rejected, ProposalStatus::Approved, Role::TenantAdmin).is_err());
    }

    #[test]
    fn test_idempotency_key_deterministic() {
        let k1 = compute_idempotency_key(&["acme", "support", "C12", "v3.2.0", "split_v1"]);
        let k2 = compute_idempotency_key(&["acme", "support", "C12", "v3.2.0", "split_v1"]);
        assert_eq!(k1, k2);
        let k3 = compute_idempotency_key(&["acme", "support", "C13", "v3.2.0", "split_v1"]);
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_role_can_review() {
        assert!(Role::CurationReviewer.can_review());
        assert!(Role::TaxonomyAdmin.can_review());
        assert!(!Role::Viewer.can_review());
        assert!(!Role::CurationProposer.can_review());
    }

    #[test]
    fn test_role_can_promote() {
        assert!(Role::CurationPromoter.can_promote());
        assert!(Role::TenantAdmin.can_promote());
        assert!(!Role::CurationReviewer.can_promote());
    }
}
