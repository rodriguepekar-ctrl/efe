//! Taxonomy version DAG manager.
//!
//! Models taxonomy versions as a directed acyclic graph with parent-child
//! relationships, supporting fork/merge semantics. Enforces invariants :
//!   - parent must exist (FK)
//!   - parent must be same (tenant, catalog)
//!   - no cycles
//!   - semver monotonically increasing from parent
//!   - only one ACTIVE version per (tenant, catalog) — DB partial index
//!
//! See § 11 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{CatalogId, CurationError, CurationResult, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "taxonomy_version_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VersionStatus {
    Draft, Candidate, Active, Deprecated, RolledBack,
}

impl VersionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "DRAFT", Self::Candidate => "CANDIDATE",
            Self::Active => "ACTIVE", Self::Deprecated => "DEPRECATED",
            Self::RolledBack => "ROLLED_BACK",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyVersion {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub semver: String,
    pub parent_id: Option<Uuid>,
    pub status: VersionStatus,
    pub proposal_id: Option<Uuid>,
    pub anchors_json: serde_json::Value,
    pub cosign_signature_url: Option<String>,
    pub cosign_cert_url: Option<String>,
    pub rekor_log_index: Option<i64>,
    pub canary_started_at: Option<DateTime<Utc>>,
    pub canary_completed_at: Option<DateTime<Utc>>,
    pub canary_metrics_json: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub rolled_back_at: Option<DateTime<Utc>>,
}

/// Parsed semver triple (major, minor, patch). Pre-release suffixes are
/// preserved as a string for lexicographic compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<String>,
}

impl SemVer {
    pub fn parse(s: &str) -> CurationResult<Self> {
        let (core, pre) = s.split_once('-').map(|(a, b)| (a, Some(b.to_string()))).unwrap_or((s, None));
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 { return Err(CurationError::InvalidSemver(s.into())); }
        let major: u32 = parts[0].parse().map_err(|_| CurationError::InvalidSemver(s.into()))?;
        let minor: u32 = parts[1].parse().map_err(|_| CurationError::InvalidSemver(s.into()))?;
        let patch: u32 = parts[2].parse().map_err(|_| CurationError::InvalidSemver(s.into()))?;
        Ok(Self { major, minor, patch, pre })
    }
    pub fn is_strictly_greater(&self, other: &SemVer) -> bool {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            _ => {}
        }
        match self.minor.cmp(&other.minor) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            _ => {}
        }
        match self.patch.cmp(&other.patch) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            _ => {}
        }
        // Same core ; pre-release is "less than" no-pre. Rare in M23 but supported.
        match (&self.pre, &other.pre) {
            (None, Some(_)) => true,           // 1.0.0 > 1.0.0-beta
            (Some(_), None) => false,
            (Some(a), Some(b)) => a > b,
            (None, None) => false,
        }
    }
}

/// Validate insertion of a new taxonomy version. Run inside the insert tx.
pub async fn validate_insertion(
    tx: &mut Transaction<'_, Postgres>, new: &TaxonomyVersion,
) -> CurationResult<()> {
    // Parent existence + tenant boundary
    if let Some(parent_id) = new.parent_id {
        let parent = sqlx::query_as::<_, TaxonomyVersionRow>(
            "SELECT * FROM taxonomy_version WHERE id = $1",
        )
        .bind(parent_id)
        .fetch_optional(&mut **tx).await?
        .ok_or(CurationError::ParentVersionNotFound(parent_id))?;
        let parent: TaxonomyVersion = parent.into();
        if parent.tenant_id != new.tenant_id || parent.catalog_id != new.catalog_id {
            return Err(CurationError::CrossTenantParent {
                parent_tenant: parent.tenant_id.0, new_tenant: new.tenant_id.0.clone(),
            });
        }
        // Semver monotonic from parent
        let parent_sv = SemVer::parse(&parent.semver)?;
        let new_sv = SemVer::parse(&new.semver)?;
        if !new_sv.is_strictly_greater(&parent_sv) {
            return Err(CurationError::SemverNotIncreasing {
                new: new.semver.clone(), parent: parent.semver,
            });
        }
        // Cycle check : ensure new.id isn't already an ancestor of parent.
        if has_path(&mut **tx, new.id, parent_id).await? {
            return Err(CurationError::DagCycle { parent: parent_id, child: new.id });
        }
    }
    // Multi-active enforcement is via DB partial unique index ; we rely on it.
    Ok(())
}

/// Check whether `from` has a path to `to` via parent_id chain.
async fn has_path(tx: &mut sqlx::PgConnection, from: Uuid, to: Uuid) -> CurationResult<bool> {
    if from == to { return Ok(true); }
    let mut current = Some(from);
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id) { return Ok(false); }  // cycle in existing data ; defensive
        let parent_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT parent_id FROM taxonomy_version WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx).await?
        .flatten();
        if parent_id == Some(to) { return Ok(true); }
        current = parent_id;
    }
    Ok(false)
}

/// Insert a taxonomy version after validation.
pub async fn insert_version(
    tx: &mut Transaction<'_, Postgres>, v: &TaxonomyVersion,
) -> CurationResult<()> {
    validate_insertion(tx, v).await?;
    sqlx::query(
        r#"
        INSERT INTO taxonomy_version (
            id, tenant_id, catalog_id, semver, parent_id,
            status, proposal_id, anchors_json,
            cosign_signature_url, cosign_cert_url, rekor_log_index,
            canary_started_at, canary_completed_at, canary_metrics_json,
            created_at, promoted_at, rolled_back_at
        ) VALUES (
            $1, $2, $3, $4, $5,
            $6::taxonomy_version_status, $7, $8,
            $9, $10, $11,
            $12, $13, $14,
            $15, $16, $17
        )
        "#,
    )
    .bind(v.id).bind(&v.tenant_id.0).bind(&v.catalog_id.0)
    .bind(&v.semver).bind(v.parent_id)
    .bind(v.status.as_str()).bind(v.proposal_id).bind(&v.anchors_json)
    .bind(&v.cosign_signature_url).bind(&v.cosign_cert_url).bind(v.rekor_log_index)
    .bind(v.canary_started_at).bind(v.canary_completed_at).bind(&v.canary_metrics_json)
    .bind(v.created_at).bind(v.promoted_at).bind(v.rolled_back_at)
    .execute(&mut **tx).await?;
    Ok(())
}

/// Get the currently ACTIVE version for a (tenant, catalog).
pub async fn get_active_version(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId,
) -> CurationResult<Option<TaxonomyVersion>> {
    let row = sqlx::query_as::<_, TaxonomyVersionRow>(
        r#"SELECT * FROM taxonomy_version
           WHERE tenant_id = $1 AND catalog_id = $2 AND status = 'ACTIVE'
           LIMIT 1"#,
    )
    .bind(&tenant.0).bind(&catalog.0)
    .fetch_optional(pg).await?;
    Ok(row.map(Into::into))
}

/// List versions for a (tenant, catalog), most-recent first.
pub async fn list_versions(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, limit: i64,
) -> CurationResult<Vec<TaxonomyVersion>> {
    let rows = sqlx::query_as::<_, TaxonomyVersionRow>(
        r#"SELECT * FROM taxonomy_version
           WHERE tenant_id = $1 AND catalog_id = $2
           ORDER BY created_at DESC
           LIMIT $3"#,
    )
    .bind(&tenant.0).bind(&catalog.0).bind(limit)
    .fetch_all(pg).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct TaxonomyVersionRow {
    id: Uuid,
    tenant_id: String,
    catalog_id: String,
    semver: String,
    parent_id: Option<Uuid>,
    status: VersionStatus,
    proposal_id: Option<Uuid>,
    anchors_json: serde_json::Value,
    cosign_signature_url: Option<String>,
    cosign_cert_url: Option<String>,
    rekor_log_index: Option<i64>,
    canary_started_at: Option<DateTime<Utc>>,
    canary_completed_at: Option<DateTime<Utc>>,
    canary_metrics_json: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
    promoted_at: Option<DateTime<Utc>>,
    rolled_back_at: Option<DateTime<Utc>>,
}

impl From<TaxonomyVersionRow> for TaxonomyVersion {
    fn from(r: TaxonomyVersionRow) -> Self {
        Self {
            id: r.id, tenant_id: TenantId(r.tenant_id), catalog_id: CatalogId(r.catalog_id),
            semver: r.semver, parent_id: r.parent_id,
            status: r.status, proposal_id: r.proposal_id, anchors_json: r.anchors_json,
            cosign_signature_url: r.cosign_signature_url,
            cosign_cert_url: r.cosign_cert_url, rekor_log_index: r.rekor_log_index,
            canary_started_at: r.canary_started_at, canary_completed_at: r.canary_completed_at,
            canary_metrics_json: r.canary_metrics_json, created_at: r.created_at,
            promoted_at: r.promoted_at, rolled_back_at: r.rolled_back_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parse_simple() {
        let v = SemVer::parse("3.2.1").unwrap();
        assert_eq!(v.major, 3); assert_eq!(v.minor, 2); assert_eq!(v.patch, 1);
        assert!(v.pre.is_none());
    }

    #[test]
    fn semver_parse_with_pre() {
        let v = SemVer::parse("1.0.0-beta.2").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.pre.as_deref(), Some("beta.2"));
    }

    #[test]
    fn semver_invalid() {
        assert!(SemVer::parse("3.2").is_err());
        assert!(SemVer::parse("v3.2.1").is_err());
        assert!(SemVer::parse("3.2.x").is_err());
    }

    #[test]
    fn semver_strictly_greater_major() {
        let a = SemVer::parse("4.0.0").unwrap();
        let b = SemVer::parse("3.99.99").unwrap();
        assert!(a.is_strictly_greater(&b));
    }

    #[test]
    fn semver_strictly_greater_patch() {
        let a = SemVer::parse("1.0.1").unwrap();
        let b = SemVer::parse("1.0.0").unwrap();
        assert!(a.is_strictly_greater(&b));
    }

    #[test]
    fn semver_pre_less_than_release() {
        let a = SemVer::parse("1.0.0").unwrap();
        let b = SemVer::parse("1.0.0-beta").unwrap();
        assert!(a.is_strictly_greater(&b));
    }

    #[test]
    fn semver_equal_not_greater() {
        let a = SemVer::parse("1.0.0").unwrap();
        let b = SemVer::parse("1.0.0").unwrap();
        assert!(!a.is_strictly_greater(&b));
    }
}
