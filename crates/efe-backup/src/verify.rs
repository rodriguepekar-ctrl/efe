//! Sprint D17 — post-restore verification suite.
//!
//! Cf. architecture § 10.3.
//!
//! After a restore completes, every Verifier in the configured
//! suite is run sequentially. The aggregate outcome decides
//! whether smoke tests can proceed.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::error::BackupError;

/// Symbolic kind of verifier. Stable strings used in metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifierKind {
    /// HMAC chain integrity (D14-bis full re-verify).
    AuditChain,
    /// Bundle Ed25519 signature verification (D16).
    BundleSignatures,
    /// OIDC issuers reachability (D15 JWKS fetch).
    OidcIssuers,
    /// `efe-server catalog list` smoke test.
    CatalogSmoke,
    /// `efe-server execute` smoke test.
    ExecuteSmoke,
}

impl VerifierKind {
    /// Stable label.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuditChain => "audit_chain",
            Self::BundleSignatures => "bundle_signatures",
            Self::OidcIssuers => "oidc_issuers",
            Self::CatalogSmoke => "catalog_smoke",
            Self::ExecuteSmoke => "execute_smoke",
        }
    }

    /// Iterator over all variants for the canonical suite.
    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AuditChain,
            Self::BundleSignatures,
            Self::OidcIssuers,
            Self::CatalogSmoke,
            Self::ExecuteSmoke,
        ]
        .into_iter()
    }
}

/// Outcome of one verifier run.
#[derive(Debug, Clone)]
pub struct VerifierStatus {
    /// Which check this represents.
    pub kind: VerifierKind,
    /// `true` iff the check passed.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// Aggregate status of the full suite.
#[derive(Debug, Clone, Default)]
pub struct VerifyAggregate {
    /// Per-verifier statuses, in execution order.
    pub statuses: Vec<VerifierStatus>,
}

impl VerifyAggregate {
    /// `true` iff every verifier passed.
    #[must_use]
    pub fn all_ok(&self) -> bool {
        !self.statuses.is_empty() && self.statuses.iter().all(|s| s.ok)
    }

    /// First failing verifier kind, if any.
    #[must_use]
    pub fn first_failure(&self) -> Option<&VerifierStatus> {
        self.statuses.iter().find(|s| !s.ok)
    }
}

/// Returned by an individual [`Verifier`] impl.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VerifyError {
    /// The verifier could not run (process spawn failed, network
    /// unreachable, etc.).
    #[error("verifier infrastructure error: {0}")]
    Infra(String),
    /// The verifier ran but reported a fatal anomaly.
    #[error("verifier reported anomaly: {0}")]
    Anomaly(String),
}

impl From<VerifyError> for BackupError {
    fn from(value: VerifyError) -> Self {
        BackupError::Verify(value.to_string())
    }
}

/// One concrete check.
#[async_trait]
pub trait Verifier: Send + Sync + std::fmt::Debug {
    /// Symbolic kind.
    fn kind(&self) -> VerifierKind;

    /// Run the check.
    async fn check(&self, target_env: &str) -> Result<VerifierStatus, VerifyError>;
}

/// Run every verifier in `suite` against `target_env` and collect
/// results. Stops at the first failure if `fail_fast = true`.
pub async fn run_suite(
    suite: &[Arc<dyn Verifier>],
    target_env: &str,
    fail_fast: bool,
) -> Result<VerifyAggregate, BackupError> {
    let mut agg = VerifyAggregate::default();
    for v in suite {
        match v.check(target_env).await {
            Ok(status) => {
                let failed = !status.ok;
                agg.statuses.push(status);
                if failed && fail_fast {
                    return Ok(agg);
                }
            }
            Err(e) => {
                agg.statuses.push(VerifierStatus {
                    kind: v.kind(),
                    ok: false,
                    detail: e.to_string(),
                });
                if fail_fast {
                    return Ok(agg);
                }
            }
        }
    }
    Ok(agg)
}

// ----------------------------------------------------------------------
// Mock verifier for tests
// ----------------------------------------------------------------------

/// Test-only verifier with a canned outcome.
#[derive(Debug)]
pub struct MockVerifier {
    kind: VerifierKind,
    outcome: std::sync::Mutex<Option<Result<VerifierStatus, VerifyError>>>,
}

impl MockVerifier {
    /// Construct an always-ok mock for `kind`.
    pub fn ok(kind: VerifierKind) -> Self {
        Self {
            kind,
            outcome: std::sync::Mutex::new(Some(Ok(VerifierStatus {
                kind,
                ok: true,
                detail: "mock-ok".into(),
            }))),
        }
    }

    /// Construct an always-failing mock for `kind`.
    pub fn anomaly(kind: VerifierKind, msg: impl Into<String>) -> Self {
        Self {
            kind,
            outcome: std::sync::Mutex::new(Some(Err(VerifyError::Anomaly(msg.into())))),
        }
    }
}

#[async_trait]
impl Verifier for MockVerifier {
    fn kind(&self) -> VerifierKind {
        self.kind
    }

    async fn check(
        &self,
        _target_env: &str,
    ) -> Result<VerifierStatus, VerifyError> {
        let mut g = self.outcome.lock().map_err(|_| {
            VerifyError::Infra("mock outcome mutex poisoned".into())
        })?;
        match g.take() {
            Some(r) => r,
            None => Ok(VerifierStatus {
                kind: self.kind,
                ok: true,
                detail: "default-ok".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_suite_all_ok() {
        let suite: Vec<Arc<dyn Verifier>> = vec![
            Arc::new(MockVerifier::ok(VerifierKind::AuditChain)),
            Arc::new(MockVerifier::ok(VerifierKind::BundleSignatures)),
            Arc::new(MockVerifier::ok(VerifierKind::OidcIssuers)),
            Arc::new(MockVerifier::ok(VerifierKind::CatalogSmoke)),
            Arc::new(MockVerifier::ok(VerifierKind::ExecuteSmoke)),
        ];
        let agg = run_suite(&suite, "drill", true).await.unwrap();
        assert_eq!(agg.statuses.len(), 5);
        assert!(agg.all_ok());
    }

    #[tokio::test]
    async fn run_suite_fail_fast_stops_at_first_failure() {
        let suite: Vec<Arc<dyn Verifier>> = vec![
            Arc::new(MockVerifier::ok(VerifierKind::AuditChain)),
            Arc::new(MockVerifier::anomaly(
                VerifierKind::BundleSignatures,
                "tampered",
            )),
            Arc::new(MockVerifier::ok(VerifierKind::OidcIssuers)),
        ];
        let agg = run_suite(&suite, "drill", true).await.unwrap();
        // 2 statuses : the ok one + the failure that triggered the
        // fail-fast.
        assert_eq!(agg.statuses.len(), 2);
        assert!(!agg.all_ok());
        let f = agg.first_failure().unwrap();
        assert_eq!(f.kind, VerifierKind::BundleSignatures);
    }

    #[tokio::test]
    async fn run_suite_no_fail_fast_runs_all() {
        let suite: Vec<Arc<dyn Verifier>> = vec![
            Arc::new(MockVerifier::ok(VerifierKind::AuditChain)),
            Arc::new(MockVerifier::anomaly(
                VerifierKind::BundleSignatures,
                "tampered",
            )),
            Arc::new(MockVerifier::ok(VerifierKind::OidcIssuers)),
        ];
        let agg = run_suite(&suite, "drill", false).await.unwrap();
        assert_eq!(agg.statuses.len(), 3);
        assert!(!agg.all_ok());
    }

    #[test]
    fn verifier_kind_all_lists_all_5() {
        let all: Vec<_> = VerifierKind::all().collect();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn verifier_kind_strings_are_canonical() {
        assert_eq!(VerifierKind::AuditChain.as_str(), "audit_chain");
        assert_eq!(VerifierKind::BundleSignatures.as_str(), "bundle_signatures");
        assert_eq!(VerifierKind::OidcIssuers.as_str(), "oidc_issuers");
        assert_eq!(VerifierKind::CatalogSmoke.as_str(), "catalog_smoke");
        assert_eq!(VerifierKind::ExecuteSmoke.as_str(), "execute_smoke");
    }

    #[test]
    fn aggregate_all_ok_false_when_empty() {
        let agg = VerifyAggregate::default();
        assert!(!agg.all_ok());
    }

    #[tokio::test]
    async fn verify_error_converts_to_backup_error() {
        let e = VerifyError::Anomaly("oops".into());
        let be: BackupError = e.into();
        assert!(matches!(be, BackupError::Verify(_)));
    }
}
