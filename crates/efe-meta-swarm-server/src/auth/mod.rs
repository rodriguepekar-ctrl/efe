//! Authentication + authorisation traits.
//!
//! PR #6 ships stubs only ; PR #16 wires the real mTLS + JWT impls.

pub mod bearer;
pub mod mtls;

use async_trait::async_trait;

/// Outcome of an auth check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// Caller is authorized.
    Allow,
    /// Caller is rejected ; the message is human-readable.
    Deny(String),
}

/// Caller identity, opaque to the service.  In PR #16 this carries
/// the SAN from the leaf cert + the JWT claims.
#[derive(Debug, Clone, Default)]
pub struct CallerIdentity {
    pub san: Option<String>,
    pub scopes: Vec<String>,
}

impl CallerIdentity {
    /// Build a [`CallerIdentity`] for an anonymous local caller (dev
    /// mode).  Real PR #16 will derive this from the TLS handshake.
    #[must_use]
    pub fn anonymous_local() -> Self {
        Self::default()
    }
}

/// Auth-z policy.
#[async_trait]
pub trait Authorizer: Send + Sync + 'static {
    /// Check whether `caller` is allowed to invoke `rpc_name`.
    async fn authorize(&self, caller: &CallerIdentity, rpc_name: &str) -> AuthDecision;
}

/// Stub authorizer : allows everything.  ONLY for dev / PR #6
/// scaffold.  Replaced by `MtlsAuthorizer` in PR #16.
#[derive(Debug, Default, Clone)]
pub struct AllowAll;

#[async_trait]
impl Authorizer for AllowAll {
    async fn authorize(&self, _: &CallerIdentity, _: &str) -> AuthDecision {
        AuthDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allow_all_allows() {
        let a = AllowAll;
        let id = CallerIdentity::anonymous_local();
        assert_eq!(a.authorize(&id, "Iterate").await, AuthDecision::Allow);
    }
}
