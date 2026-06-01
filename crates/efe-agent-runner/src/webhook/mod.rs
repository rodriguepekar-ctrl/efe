//! M26 — admission webhook server for `AgentRun` + `AgentRunTemplate` CRs.
//!
//! Runs as a sidecar HTTPS endpoint inside the controller pod (port 8443).
//! cert-manager issues + auto-rotates the TLS cert. The webhook validates
//! AdmissionReview requests against the M26 production rules + reads
//! tenant flags from the DB.
//!
//! See `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` § 8.

pub mod server;
pub mod tls;
pub mod validators;

use crate::profile::ProfileRegistry;
use sqlx::PgPool;
use std::sync::Arc;

/// Shared state for webhook handlers.
#[derive(Clone)]
pub struct WebhookState {
    pub pg: PgPool,
    pub profiles: Arc<ProfileRegistry>,
}

/// Validation outcome for a single CR.
#[derive(Debug)]
pub enum ValidationOutcome {
    /// Allow the CR to enter etcd.
    Allow,
    /// Reject with the given message (returned to the K8s API client).
    Deny(String),
}

impl ValidationOutcome {
    pub fn deny<S: Into<String>>(msg: S) -> Self {
        Self::Deny(msg.into())
    }
}
