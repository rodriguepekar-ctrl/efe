// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `AppState` — § 7.3.
//!
//! Holds the `Arc<dyn ...>` references to every subsystem the
//! gateway routes traffic to.  Cloneable (cheap, Arc-only).

use std::sync::Arc;

use efe_agentic_bridge::AgenticBridge;
use efe_compliance_wiring::WiredComplianceRunner;
use efe_gold_test_gate::GoldApi;
use efe_llm_provenance::LlmProvenanceStore;
use efe_publisher_identity::PublisherApi;
use efe_swarm_orchestrator::SwarmOrchestrator;

use crate::auth::JwtValidator;
use crate::observability::N22GatewayMetrics;

/// **N22 + N24** : the shared application state.
///
/// Clone is cheap (all fields are `Arc<...>`).  The `compliance`
/// field (N24) is `Option<Arc<...>>` so existing N22-era tests
/// can build an `AppState` without bringing up the wiring stack.
#[derive(Clone)]
pub struct AppState {
    pub swarm: Arc<SwarmOrchestrator>,
    pub agentic: Arc<dyn AgenticBridge>,
    pub publisher_api: Arc<dyn PublisherApi>,
    pub gold_api: Arc<dyn GoldApi>,
    pub jwt_validator: Arc<dyn JwtValidator>,
    pub metrics: Arc<N22GatewayMetrics>,
    /// **N24** : optional wired compliance runner.  `None` →
    /// the default DAG uses `experimental_fast` policy (N23
    /// behaviour).  `Some(_)` → tier1_strict is selected per
    /// stability_tier (see `dag_registry`).
    pub compliance: Option<Arc<WiredComplianceRunner>>,
    /// **N26 + G13** : optional N17 provenance store handle.
    /// When `Some`, the `GET /v1/llm-provenance` operator
    /// endpoint queries this store ; when `None`, the endpoint
    /// returns 503.
    pub llm_provenance_store: Option<Arc<dyn LlmProvenanceStore>>,
    /// **Sprint 19 — N27 + § 17.1 + § 17.5** : master kill-switch
    /// for the streaming endpoint.  Read from
    /// `EFE_BRIDGE_STREAM_ENABLED` at boot time.  When `false`,
    /// `POST /v1/agentic/call-stream` returns 404.  Defaults to
    /// `true` (streaming enabled).
    pub stream_endpoint_enabled: bool,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl AppState {
    /// **N22** : Convenience builder for tests.
    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::default()
    }
}

pub struct AppStateBuilder {
    swarm: Option<Arc<SwarmOrchestrator>>,
    agentic: Option<Arc<dyn AgenticBridge>>,
    publisher_api: Option<Arc<dyn PublisherApi>>,
    gold_api: Option<Arc<dyn GoldApi>>,
    jwt_validator: Option<Arc<dyn JwtValidator>>,
    metrics: Option<Arc<N22GatewayMetrics>>,
    compliance: Option<Arc<WiredComplianceRunner>>,
    llm_provenance_store: Option<Arc<dyn LlmProvenanceStore>>,
    /// **N27** : streaming endpoint kill-switch ; defaults to
    /// `true` (enabled).  Toggled at boot from env in `main.rs`.
    stream_endpoint_enabled: bool,
}

impl Default for AppStateBuilder {
    fn default() -> Self {
        Self {
            swarm: None,
            agentic: None,
            publisher_api: None,
            gold_api: None,
            jwt_validator: None,
            metrics: None,
            compliance: None,
            llm_provenance_store: None,
            // **N27** : streaming enabled by default.
            stream_endpoint_enabled: true,
        }
    }
}

impl AppStateBuilder {
    pub fn with_swarm(mut self, s: Arc<SwarmOrchestrator>) -> Self {
        self.swarm = Some(s);
        self
    }
    pub fn with_agentic(mut self, a: Arc<dyn AgenticBridge>) -> Self {
        self.agentic = Some(a);
        self
    }
    pub fn with_publisher_api(mut self, p: Arc<dyn PublisherApi>) -> Self {
        self.publisher_api = Some(p);
        self
    }
    pub fn with_gold_api(mut self, g: Arc<dyn GoldApi>) -> Self {
        self.gold_api = Some(g);
        self
    }
    pub fn with_jwt_validator(mut self, v: Arc<dyn JwtValidator>) -> Self {
        self.jwt_validator = Some(v);
        self
    }
    pub fn with_metrics(mut self, m: Arc<N22GatewayMetrics>) -> Self {
        self.metrics = Some(m);
        self
    }

    /// **N24** : wire the compliance runner.  Optional ; absence
    /// preserves N23 behaviour (default DAG runs under
    /// `experimental_fast`).
    pub fn with_compliance(mut self, c: Arc<WiredComplianceRunner>) -> Self {
        self.compliance = Some(c);
        self
    }

    /// **N26 + G13** : wire the N17 provenance store handle so
    /// `GET /v1/llm-provenance` can query records.  Optional ;
    /// absence makes the operator endpoint return 503.
    pub fn with_llm_provenance_store(mut self, s: Arc<dyn LlmProvenanceStore>) -> Self {
        self.llm_provenance_store = Some(s);
        self
    }

    /// **N27 + § 17.1 + § 17.5** : toggle the streaming endpoint
    /// at the gateway level.  When `false`, every request to
    /// `POST /v1/agentic/call-stream` short-circuits with 404.
    /// Defaults to `true`.  `main.rs` reads
    /// `EFE_BRIDGE_STREAM_ENABLED` and calls this once at boot.
    pub fn with_stream_endpoint_enabled(mut self, enabled: bool) -> Self {
        self.stream_endpoint_enabled = enabled;
        self
    }

    pub fn build(self) -> Result<AppState, String> {
        Ok(AppState {
            swarm: self.swarm.ok_or("missing swarm")?,
            agentic: self.agentic.ok_or("missing agentic")?,
            publisher_api: self.publisher_api.ok_or("missing publisher_api")?,
            gold_api: self.gold_api.ok_or("missing gold_api")?,
            jwt_validator: self.jwt_validator.ok_or("missing jwt_validator")?,
            metrics: self
                .metrics
                .unwrap_or_default(),
            compliance: self.compliance,
            llm_provenance_store: self.llm_provenance_store,
            stream_endpoint_enabled: self.stream_endpoint_enabled,
        })
    }
}
