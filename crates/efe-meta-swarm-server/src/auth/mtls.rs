//! mTLS authorizer — STUB in PR #6.  Real impl in PR #16.

use async_trait::async_trait;

use super::{AuthDecision, Authorizer, CallerIdentity};

#[derive(Debug, Clone, Default)]
pub struct MtlsAuthorizer;

#[async_trait]
impl Authorizer for MtlsAuthorizer {
    async fn authorize(&self, _: &CallerIdentity, _: &str) -> AuthDecision {
        AuthDecision::Deny("mtls authorizer not yet wired (PR #16)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mtls_stub_denies_with_pr16_message() {
        let a = MtlsAuthorizer;
        match a.authorize(&CallerIdentity::default(), "Iterate").await {
            AuthDecision::Deny(msg) => assert!(msg.contains("PR #16")),
            AuthDecision::Allow => panic!("stub should deny"),
        }
    }
}
