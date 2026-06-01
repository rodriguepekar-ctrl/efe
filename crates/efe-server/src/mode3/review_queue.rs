//! Review queue — Postgres-backed pending Mode-3 human reviews.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use super::concept_stack::ConceptStack;
use super::error::Mode3Error;

/// Mode-3 review status lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl ReviewStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pending" => Some(ReviewStatus::Pending),
            "approved" => Some(ReviewStatus::Approved),
            "rejected" => Some(ReviewStatus::Rejected),
            "expired" => Some(ReviewStatus::Expired),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewStatus::Pending => "pending",
            ReviewStatus::Approved => "approved",
            ReviewStatus::Rejected => "rejected",
            ReviewStatus::Expired => "expired",
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, ReviewStatus::Pending)
    }
}

/// Review record matching `mode3_review_queue` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub review_token: String,
    pub tenant_id: String,
    pub audit_id: i64,
    pub catalog: String,
    pub bundle_version: String,
    pub sentence_hash: String,
    pub proposed_label: String,
    pub proposed_confidence: f32,
    pub concept_stack: ConceptStack,
    pub status: ReviewStatus,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub reviewed_by: Option<String>,
    pub review_notes: Option<String>,
    pub final_label: Option<String>,
}

impl ReviewRecord {
    /// Generate a UUID v4 review_token.
    pub fn generate_token() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Build expires_at from now() + ttl_days.
    pub fn compute_expires_at(ttl_days: u32) -> DateTime<Utc> {
        Utc::now() + Duration::days(ttl_days as i64)
    }

    /// Validate token format (UUID v4 string).
    pub fn validate_token(token: &str) -> Result<(), Mode3Error> {
        uuid::Uuid::from_str(token)
            .map_err(|_| Mode3Error::Infrastructure(format!("invalid review_token: {}", token)))?;
        Ok(())
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status, ReviewStatus::Pending) && !self.is_expired()
    }
}

/// Trait abstracting Postgres review queue operations for unit
/// testability.
#[async_trait::async_trait]
pub trait ReviewQueue: Send + Sync {
    /// Insert a new pending review record. Returns the record (with
    /// generated token + expires_at populated).
    async fn insert(
        &self,
        tenant_id: &str,
        audit_id: i64,
        catalog: &str,
        bundle_version: &str,
        sentence_hash: &str,
        proposed_label: &str,
        proposed_confidence: f32,
        concept_stack: &ConceptStack,
        ttl_days: u32,
    ) -> Result<ReviewRecord, Mode3Error>;

    /// Read a single record by token.
    async fn get(&self, review_token: &str) -> Result<Option<ReviewRecord>, Mode3Error>;

    /// List records for a tenant with optional status filter.
    async fn list(
        &self,
        tenant_id: &str,
        status: Option<ReviewStatus>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<ReviewRecord>, Mode3Error>;

    /// Approve a pending review. Atomic, idempotent.
    async fn approve(
        &self,
        review_token: &str,
        final_label: Option<&str>,
        reviewed_by: &str,
        review_notes: Option<&str>,
    ) -> Result<ReviewRecord, Mode3Error>;

    /// Reject a pending review. Atomic.
    async fn reject(
        &self,
        review_token: &str,
        reason: &str,
        reviewed_by: &str,
    ) -> Result<ReviewRecord, Mode3Error>;

    /// Mark stale pending reviews as expired (cron job).
    async fn expire_stale(&self) -> Result<u64, Mode3Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_uuid_v4() {
        let token = ReviewRecord::generate_token();
        assert_eq!(token.len(), 36); // UUID v4 length
        let uuid = uuid::Uuid::from_str(&token).unwrap();
        assert_eq!(uuid.get_version_num(), 4);
    }

    #[test]
    fn test_generate_token_unique() {
        let t1 = ReviewRecord::generate_token();
        let t2 = ReviewRecord::generate_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_validate_token_accepts_uuid_v4() {
        let token = ReviewRecord::generate_token();
        assert!(ReviewRecord::validate_token(&token).is_ok());
    }

    #[test]
    fn test_validate_token_rejects_garbage() {
        assert!(ReviewRecord::validate_token("not-a-uuid").is_err());
        assert!(ReviewRecord::validate_token("").is_err());
    }

    #[test]
    fn test_compute_expires_at_7_days() {
        let now = Utc::now();
        let expires = ReviewRecord::compute_expires_at(7);
        let diff = expires - now;
        assert!(diff.num_days() >= 6);
        assert!(diff.num_days() <= 7);
    }

    #[test]
    fn test_review_status_parse() {
        assert_eq!(ReviewStatus::parse("pending"), Some(ReviewStatus::Pending));
        assert_eq!(
            ReviewStatus::parse("APPROVED"),
            Some(ReviewStatus::Approved)
        );
        assert_eq!(
            ReviewStatus::parse("rejected"),
            Some(ReviewStatus::Rejected)
        );
        assert_eq!(ReviewStatus::parse("EXPIRED"), Some(ReviewStatus::Expired));
        assert_eq!(ReviewStatus::parse("nonsense"), None);
    }

    #[test]
    fn test_review_status_terminal() {
        assert!(!ReviewStatus::Pending.is_terminal());
        assert!(ReviewStatus::Approved.is_terminal());
        assert!(ReviewStatus::Rejected.is_terminal());
        assert!(ReviewStatus::Expired.is_terminal());
    }
}
