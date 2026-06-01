// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : KMS key accessibility resolver.
//!
//! Architecture § 7.2 step 7 + ADR-3.  Sprint 9 ships an in-memory
//! stub used in tests + dev ; Sprint 10 ships an AWS-KMS-backed
//! implementation.

use std::collections::HashSet;

use async_trait::async_trait;
use dashmap::DashSet;
use thiserror::Error;

use efe_nano_sdk::RegionCode;

/// **Sprint 9 — N16** : closed taxonomy of KMS errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KmsError {
    /// One or more KMS ARNs are NOT accessible in the target region.
    /// Used by the coordinator to short-circuit replays that would
    /// fail later during chain decryption (architecture G12 + F6).
    #[error("KMS key(s) not accessible in {region} : {arns:?}")]
    InsufficientPermissions {
        region: RegionCode,
        arns: Vec<String>,
    },

    /// Backend (AWS API) error.
    #[error("KMS backend error : {0}")]
    Backend(String),
}

/// **Sprint 9 — N16** : check accessibility of KMS keys in a target
/// region.
#[async_trait]
pub trait KmsKeyResolver: Send + Sync + 'static {
    /// **N16** : Check that `kms_arn` is accessible in `target_region`.
    async fn check_access(
        &self,
        target_region: RegionCode,
        kms_arn: &str,
    ) -> Result<(), KmsError>;

    /// **N16** : Bulk variant.  Returns the subset of `arns` that
    /// are NOT accessible.  Empty Vec → all accessible.
    async fn check_access_bulk(
        &self,
        target_region: RegionCode,
        arns: &[String],
    ) -> Result<Vec<String>, KmsError> {
        let mut missing = Vec::new();
        for arn in arns {
            if let Err(KmsError::InsufficientPermissions { .. }) =
                self.check_access(target_region, arn).await
            {
                missing.push(arn.clone());
            }
        }
        Ok(missing)
    }
}

/// **Sprint 9 — N16** : in-memory implementation.  Operators
/// register `(region, kms_arn)` pairs as "accessible" ; everything
/// else is denied by default.
pub struct InMemoryKmsKeyResolver {
    /// (region, arn) pairs known accessible.
    accessible: DashSet<(RegionCode, String)>,
    /// If `true`, every check returns Ok (test convenience).
    permissive: bool,
}

impl std::fmt::Debug for InMemoryKmsKeyResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryKmsKeyResolver")
            .field("permissive", &self.permissive)
            .field("accessible_count", &self.accessible.len())
            .finish()
    }
}

impl Default for InMemoryKmsKeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryKmsKeyResolver {
    /// **N16** : Construct a default resolver — everything denied
    /// unless explicitly granted.
    pub fn new() -> Self {
        Self {
            accessible: DashSet::new(),
            permissive: false,
        }
    }

    /// **N16** : Construct a permissive resolver — every check
    /// returns Ok.  Used by tests that don't care about KMS.
    pub fn permissive() -> Self {
        Self {
            accessible: DashSet::new(),
            permissive: true,
        }
    }

    /// **N16** : Grant cross-region access for `(region, kms_arn)`.
    pub fn grant(&self, region: RegionCode, kms_arn: impl Into<String>) {
        self.accessible.insert((region, kms_arn.into()));
    }

    /// **N16** : Revoke access.
    pub fn revoke(&self, region: RegionCode, kms_arn: &str) {
        let key = (region, kms_arn.to_string());
        self.accessible.remove(&key);
    }

    /// **N16** : Number of granted (region, arn) pairs.
    pub fn grant_count(&self) -> usize {
        self.accessible.len()
    }
}

#[async_trait]
impl KmsKeyResolver for InMemoryKmsKeyResolver {
    async fn check_access(
        &self,
        target_region: RegionCode,
        kms_arn: &str,
    ) -> Result<(), KmsError> {
        if self.permissive {
            return Ok(());
        }
        let key = (target_region, kms_arn.to_string());
        if self.accessible.contains(&key) {
            Ok(())
        } else {
            Err(KmsError::InsufficientPermissions {
                region: target_region,
                arns: vec![kms_arn.to_string()],
            })
        }
    }

    async fn check_access_bulk(
        &self,
        target_region: RegionCode,
        arns: &[String],
    ) -> Result<Vec<String>, KmsError> {
        if self.permissive {
            return Ok(vec![]);
        }
        // Build a HashSet of owned strings for the target_region.
        let granted: HashSet<String> = self
            .accessible
            .iter()
            .filter(|r| r.0 == target_region)
            .map(|r| r.1.clone())
            .collect();
        let mut missing = Vec::new();
        for arn in arns {
            if !granted.contains(arn) {
                missing.push(arn.clone());
            }
        }
        Ok(missing)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unknown_arn_denied_by_default() {
        let r = InMemoryKmsKeyResolver::new();
        let err = r
            .check_access(RegionCode::UsEast1, "arn:aws:kms:us-east-1::test")
            .await
            .unwrap_err();
        assert!(matches!(err, KmsError::InsufficientPermissions { .. }));
    }

    #[tokio::test]
    async fn granted_arn_accessible() {
        let r = InMemoryKmsKeyResolver::new();
        let arn = "arn:aws:kms:eu-west-1::tenant-x";
        r.grant(RegionCode::EuWest1, arn);
        assert!(r.check_access(RegionCode::EuWest1, arn).await.is_ok());
    }

    #[tokio::test]
    async fn grant_is_region_scoped() {
        let r = InMemoryKmsKeyResolver::new();
        let arn = "arn:aws:kms:eu-west-1::tenant-x";
        r.grant(RegionCode::EuWest1, arn);
        // Same arn in different region must be denied.
        let err = r.check_access(RegionCode::UsEast1, arn).await.unwrap_err();
        assert!(matches!(err, KmsError::InsufficientPermissions { .. }));
    }

    #[tokio::test]
    async fn revoke_disables_access() {
        let r = InMemoryKmsKeyResolver::new();
        let arn = "arn:test";
        r.grant(RegionCode::UsEast1, arn);
        assert!(r.check_access(RegionCode::UsEast1, arn).await.is_ok());
        r.revoke(RegionCode::UsEast1, arn);
        let err = r.check_access(RegionCode::UsEast1, arn).await.unwrap_err();
        assert!(matches!(err, KmsError::InsufficientPermissions { .. }));
    }

    #[tokio::test]
    async fn permissive_allows_anything() {
        let r = InMemoryKmsKeyResolver::permissive();
        assert!(r
            .check_access(RegionCode::ApSoutheast1, "arbitrary-arn")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn bulk_check_returns_missing_arns() {
        let r = InMemoryKmsKeyResolver::new();
        r.grant(RegionCode::EuWest1, "arn:1");
        r.grant(RegionCode::EuWest1, "arn:2");
        let arns = vec!["arn:1".into(), "arn:2".into(), "arn:3".into()];
        let missing = r.check_access_bulk(RegionCode::EuWest1, &arns).await.unwrap();
        assert_eq!(missing, vec!["arn:3".to_string()]);
    }

    #[tokio::test]
    async fn bulk_check_empty_when_all_granted() {
        let r = InMemoryKmsKeyResolver::new();
        r.grant(RegionCode::UsEast1, "a");
        r.grant(RegionCode::UsEast1, "b");
        let arns = vec!["a".into(), "b".into()];
        let missing = r.check_access_bulk(RegionCode::UsEast1, &arns).await.unwrap();
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn bulk_check_permissive_always_empty() {
        let r = InMemoryKmsKeyResolver::permissive();
        let arns = vec!["x".into(), "y".into()];
        let missing = r.check_access_bulk(RegionCode::UsEast1, &arns).await.unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn grant_count_reflects_state() {
        let r = InMemoryKmsKeyResolver::new();
        assert_eq!(r.grant_count(), 0);
        r.grant(RegionCode::EuWest1, "a");
        r.grant(RegionCode::EuWest1, "b");
        assert_eq!(r.grant_count(), 2);
    }

    #[test]
    fn debug_format_does_not_panic() {
        let r = InMemoryKmsKeyResolver::new();
        let _ = format!("{r:?}");
    }
}
