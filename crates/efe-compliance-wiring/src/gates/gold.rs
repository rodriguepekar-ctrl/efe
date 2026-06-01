// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + G3 + I5 + F7** : `WiredGoldBindingGate` —
//! the only module that drives `GoldApi::lookup_gold_set` from
//! inside the swarm dispatch path.

use std::sync::Arc;

use async_trait::async_trait;
use efe_gold_test_gate::GoldApi;
use efe_nano_sdk::{GoldSetStatus, NanoAgent};
use efe_nano_swarm_adapter::GoldBindingGate;

use crate::error::WiringError;
use crate::metrics::N24ComplianceMetrics;

/// **N24 + G3** : concrete `GoldBindingGate` impl bound to a
/// `GoldApi` instance.  Verifies status `Approved` AND
/// `nano.gold_set_ref.gold_set_hash == registry.gold_set_hash`
/// (F7 ; defense against silent gold-set rewrites).
pub struct WiredGoldBindingGate {
    gold_api: Arc<dyn GoldApi>,
    metrics: Arc<N24ComplianceMetrics>,
}

impl std::fmt::Debug for WiredGoldBindingGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiredGoldBindingGate").finish_non_exhaustive()
    }
}

impl WiredGoldBindingGate {
    /// **N24** : construct the gate.  Callers normally do not
    /// build this directly — they go through
    /// `WiredComplianceRunner::new`.
    pub fn new(gold_api: Arc<dyn GoldApi>, metrics: Arc<N24ComplianceMetrics>) -> Self {
        Self { gold_api, metrics }
    }
}

#[async_trait]
impl GoldBindingGate for WiredGoldBindingGate {
    async fn assert_approved(&self, nano: &NanoAgent) -> Result<(), String> {
        // 1. The nano must declare a gold reference.
        let gold_ref = match nano.gold_set_ref.as_ref() {
            Some(r) => r,
            None => {
                self.metrics.gold_bound_total("no_ref");
                return Err(WiringError::GoldRefMissing.to_string());
            }
        };

        // 2. Lookup the gold set in the registry.
        let set = match self.gold_api.lookup_gold_set(gold_ref.gold_set_id).await {
            Ok(s) => s,
            Err(e) => {
                self.metrics.gold_bound_total("lookup_failed");
                return Err(WiringError::GoldLookupFailed(e.to_string()).to_string());
            }
        };

        // 3. Status MUST be Approved.
        if !matches!(set.status, GoldSetStatus::Approved) {
            self.metrics.gold_bound_total("not_approved");
            return Err(WiringError::GoldNotApproved(
                gold_set_status_label(&set.status).to_string(),
            )
            .to_string());
        }

        // 4. Hash MUST match (F7).
        if set.gold_set_hash != gold_ref.gold_set_hash {
            self.metrics.gold_bound_total("hash_drift");
            return Err(WiringError::GoldHashDrift {
                expected: gold_ref.gold_set_hash.to_hex(),
                actual: set.gold_set_hash.to_hex(),
            }
            .to_string());
        }

        self.metrics.gold_bound_total("ok");
        Ok(())
    }
}

fn gold_set_status_label(status: &GoldSetStatus) -> &'static str {
    match status {
        GoldSetStatus::Draft => "draft",
        GoldSetStatus::Approved => "approved",
        GoldSetStatus::Archived => "archived",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::N24ComplianceMetrics;
    use chrono::Utc;
    use efe_gold_test_gate::api::RegisterGoldSetRequest;
    use efe_gold_test_gate::GoldApiError;
    use efe_nano_sdk::{
        Author, Category, DeterminismLevel, ExecutionMode, GoldSetId, GoldSetRef,
        IdempotencyPolicy, NanoAgentId, NanoAgentVersion, ResourceLimits, Sha256Hash,
        StabilityTier,
    };

    fn sample_nano(gold_ref: Option<GoldSetRef>) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Extraction,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "x".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: gold_ref,
            created_at: Utc::now(),
        }
    }

    // Stub GoldApi to control responses.
    struct StubGoldApi {
        responses: std::sync::Mutex<
            std::collections::HashMap<GoldSetId, Result<efe_nano_sdk::GoldSet, GoldApiError>>,
        >,
    }

    impl StubGoldApi {
        fn new() -> Self {
            Self {
                responses: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
        fn set(&self, id: GoldSetId, response: Result<efe_nano_sdk::GoldSet, GoldApiError>) {
            self.responses.lock().unwrap().insert(id, response);
        }
    }

    #[async_trait]
    impl GoldApi for StubGoldApi {
        async fn register_gold_set(
            &self,
            _req: efe_gold_test_gate::api::RegisterGoldSetRequest,
        ) -> Result<GoldSetId, GoldApiError> {
            unimplemented!()
        }
        async fn approve_gold_set(
            &self,
            _gold_set_id: GoldSetId,
            _req: efe_gold_test_gate::api::ApproveGoldSetRequest,
        ) -> Result<efe_nano_sdk::GoldSet, GoldApiError> {
            unimplemented!()
        }
        async fn lookup_gold_set(
            &self,
            gold_set_id: GoldSetId,
        ) -> Result<efe_nano_sdk::GoldSet, GoldApiError> {
            let _ = gold_set_id;
            self.responses
                .lock()
                .unwrap()
                .get(&gold_set_id)
                .cloned()
                .unwrap_or_else(|| Err(GoldApiError::NotFound))
        }
        async fn list_gold_sets_for_agent(
            &self,
            _agent_id: &NanoAgentId,
        ) -> Result<Vec<efe_nano_sdk::GoldSet>, GoldApiError> {
            Ok(vec![])
        }
        async fn run_gate(
            &self,
            _req: efe_gold_test_gate::api::RunRequest,
        ) -> Result<efe_nano_sdk::GoldReport, GoldApiError> {
            unimplemented!()
        }
        async fn lookup_gold_run(
            &self,
            _gold_run_id: efe_nano_sdk::GoldRunId,
        ) -> Result<Option<efe_gold_test_gate::GoldRunLedgerRow>, GoldApiError> {
            unimplemented!()
        }
        async fn drift_sweep(
            &self,
            _req: efe_gold_test_gate::api::DriftSweepRequest,
        ) -> Result<efe_gold_test_gate::api::DriftSweepReport, GoldApiError> {
            unimplemented!()
        }
    }

    fn make_gold_set(
        id: GoldSetId,
        status: GoldSetStatus,
        hash: Sha256Hash,
    ) -> efe_nano_sdk::GoldSet {
        use std::time::SystemTime;
        efe_nano_sdk::GoldSet {
            gold_set_id: id,
            agent_id: NanoAgentId::new("test_agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            agent_input_schema_hash: Sha256Hash::from_bytes([0u8; 32]),
            agent_output_schema_hash: Sha256Hash::from_bytes([0u8; 32]),
            cases: vec![],
            cost_cap_usd: 10.0,
            provenance: efe_nano_sdk::Provenance::Manual,
            status,
            approvers: vec![],
            created_at: SystemTime::now(),
            approved_at: None,
            gold_set_hash: hash,
        }
    }

    #[tokio::test]
    async fn missing_gold_ref_returns_error() {
        let api = Arc::new(StubGoldApi::new());
        let gate = WiredGoldBindingGate::new(api, Arc::new(N24ComplianceMetrics::new()));
        let nano = sample_nano(None);
        let result = gate.assert_approved(&nano).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no gold_set_ref"));
    }

    #[tokio::test]
    async fn approved_with_matching_hash_passes() {
        let id = GoldSetId::new();
        let hash = Sha256Hash::from_bytes([0xABu8; 32]);
        let api = Arc::new(StubGoldApi::new());
        api.set(id, Ok(make_gold_set(id, GoldSetStatus::Approved, hash)));
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredGoldBindingGate::new(api, metrics.clone());
        let nano = sample_nano(Some(GoldSetRef {
            gold_set_id: id,
            gold_set_hash: hash,
        }));
        gate.assert_approved(&nano).await.unwrap();
        assert_eq!(metrics.snapshot().gold.ok, 1);
    }

    #[tokio::test]
    async fn not_approved_status_rejects() {
        let id = GoldSetId::new();
        let hash = Sha256Hash::from_bytes([0xABu8; 32]);
        let api = Arc::new(StubGoldApi::new());
        api.set(id, Ok(make_gold_set(id, GoldSetStatus::Draft, hash)));
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredGoldBindingGate::new(api, metrics.clone());
        let nano = sample_nano(Some(GoldSetRef {
            gold_set_id: id,
            gold_set_hash: hash,
        }));
        let result = gate.assert_approved(&nano).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("draft"));
        assert_eq!(metrics.snapshot().gold.not_approved, 1);
    }

    #[tokio::test]
    async fn hash_drift_rejects() {
        let id = GoldSetId::new();
        let api = Arc::new(StubGoldApi::new());
        let registry_hash = Sha256Hash::from_bytes([0xCDu8; 32]);
        api.set(
            id,
            Ok(make_gold_set(id, GoldSetStatus::Approved, registry_hash)),
        );
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredGoldBindingGate::new(api, metrics.clone());
        let nano_hash = Sha256Hash::from_bytes([0xEFu8; 32]);
        let nano = sample_nano(Some(GoldSetRef {
            gold_set_id: id,
            gold_set_hash: nano_hash,
        }));
        let result = gate.assert_approved(&nano).await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("hash drift"));
        assert!(msg.contains(&registry_hash.to_hex()));
        assert!(msg.contains(&nano_hash.to_hex()));
        assert_eq!(metrics.snapshot().gold.hash_drift, 1);
    }

    #[tokio::test]
    async fn lookup_error_propagates_as_lookup_failed() {
        let id = GoldSetId::new();
        let api = Arc::new(StubGoldApi::new());
        api.set(id, Err(GoldApiError::NotFound));
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredGoldBindingGate::new(api, metrics.clone());
        let nano = sample_nano(Some(GoldSetRef {
            gold_set_id: id,
            gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
        }));
        let result = gate.assert_approved(&nano).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gold set lookup failed"));
        assert_eq!(metrics.snapshot().gold.lookup_failed, 1);
    }

    #[test]
    fn status_label_helper_covers_all_variants() {
        // Compile-time exhaustiveness check : if a new variant is
        // added to GoldSetStatus the match in gold_set_status_label
        // breaks the build.
        assert_eq!(gold_set_status_label(&GoldSetStatus::Draft), "draft");
        assert_eq!(gold_set_status_label(&GoldSetStatus::Approved), "approved");
        assert_eq!(gold_set_status_label(&GoldSetStatus::Archived), "archived");
    }

    // Ensure imports don't dead-code.
    #[allow(dead_code)]
    fn _imports() {
        let _: Option<RegisterGoldSetRequest> = None;
    }
}
