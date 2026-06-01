// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + G4 + I12** : `WiredPublisherStatusGate` —
//! cheap-first dispatch gate that fails closed when the
//! publisher is suspended / revoked / sanctions-stale.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use efe_nano_sdk::{Author, NanoAgent, PublisherIdentity};
use efe_nano_swarm_adapter::PublisherStatusGate;
use efe_publisher_identity::registry::{PublisherRegistry, RegistryError};

use crate::cache::PublisherIdentityCache;
use crate::error::WiringError;
use crate::metrics::N24ComplianceMetrics;

/// **N24 + G4** : concrete `PublisherStatusGate` impl bound to a
/// `PublisherRegistry`.
pub struct WiredPublisherStatusGate {
    registry: Arc<dyn PublisherRegistry>,
    cache: Arc<PublisherIdentityCache>,
    metrics: Arc<N24ComplianceMetrics>,
    lookup_timeout: Duration,
    builtin_identity: PublisherIdentity,
}

impl std::fmt::Debug for WiredPublisherStatusGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiredPublisherStatusGate")
            .field("lookup_timeout", &self.lookup_timeout)
            .field(
                "builtin_publisher_id",
                &self.builtin_identity.publisher_id,
            )
            .finish_non_exhaustive()
    }
}

impl WiredPublisherStatusGate {
    /// **N24** : construct the gate.
    pub fn new(
        registry: Arc<dyn PublisherRegistry>,
        cache: Arc<PublisherIdentityCache>,
        metrics: Arc<N24ComplianceMetrics>,
        lookup_timeout: Duration,
        builtin_identity: PublisherIdentity,
    ) -> Self {
        Self {
            registry,
            cache,
            metrics,
            lookup_timeout,
            builtin_identity,
        }
    }

    async fn resolve(
        &self,
        publisher_id: efe_nano_sdk::PublisherId,
    ) -> Result<PublisherIdentity, WiringError> {
        if publisher_id == self.builtin_identity.publisher_id {
            return Ok(self.builtin_identity.clone());
        }
        if let Some(ident) = self.cache.get(publisher_id) {
            return Ok(ident);
        }
        let fut = self.registry.lookup(publisher_id);
        let result = tokio::time::timeout(self.lookup_timeout, fut).await;
        match result {
            Err(_) => Err(WiringError::RegistryTimeout {
                ms: self.lookup_timeout.as_millis() as u32,
            }),
            Ok(Err(RegistryError::NotFound { publisher_id })) => {
                Err(WiringError::PublisherNotFound(publisher_id))
            }
            Ok(Err(e)) => Err(WiringError::RegistryBackend(e.to_string())),
            Ok(Ok(identity)) => {
                self.cache.put(publisher_id, identity.clone());
                Ok(identity)
            }
        }
    }
}

#[async_trait]
impl PublisherStatusGate for WiredPublisherStatusGate {
    async fn assert_active(&self, nano: &NanoAgent) -> Result<(), String> {
        // Non-publisher authors skip the check (Efe / Tenant /
        // Community).  The metric counter still ticks so ops can
        // see the breakdown.
        let publisher_id = match &nano.author {
            Author::Publisher { publisher_id, .. } => *publisher_id,
            _ => {
                self.metrics.status_blocked_total("not_publisher");
                return Ok(());
            }
        };

        let identity = match self.resolve(publisher_id).await {
            Ok(id) => id,
            Err(WiringError::PublisherNotFound(p)) => {
                self.metrics.status_blocked_total("publisher_not_found");
                return Err(WiringError::PublisherNotFound(p).to_string());
            }
            Err(WiringError::RegistryTimeout { ms }) => {
                self.metrics.status_blocked_total("registry_timeout");
                return Err(WiringError::RegistryTimeout { ms }.to_string());
            }
            Err(e) => {
                self.metrics.status_blocked_total("publisher_not_found");
                return Err(e.to_string());
            }
        };

        // Cheap branch on the status enum (annex A5).
        let label = identity.status.label();
        if identity.dispatch_blocked() {
            self.metrics.status_blocked_total(label);
            return Err(WiringError::PublisherBlocked {
                publisher_id,
                reason: label.to_string(),
            }
            .to_string());
        }

        self.metrics.status_blocked_total("ok");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::PublisherIdentityCache;
    use chrono::Utc;
    use efe_nano_sdk::{
        Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, KycTier, NanoAgentId,
        NanoAgentVersion, PublisherId, PublisherKey, PublisherStatus, ResourceLimits,
        SignatureAlgorithm, StabilityTier,
    };
    use efe_publisher_identity::registry::InMemoryPublisherRegistry;
    use std::time::SystemTime;

    fn sample_nano(author: Author) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author,
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
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    fn make_gate(builtin: PublisherIdentity) -> WiredPublisherStatusGate {
        WiredPublisherStatusGate::new(
            Arc::new(InMemoryPublisherRegistry::new()),
            Arc::new(PublisherIdentityCache::default()),
            Arc::new(N24ComplianceMetrics::new()),
            Duration::from_millis(500),
            builtin,
        )
    }

    fn fake_identity(id: PublisherId, status: PublisherStatus) -> PublisherIdentity {
        PublisherIdentity {
            publisher_id: id,
            display_name: "x".into(),
            kyc_tier: KycTier::Tier2,
            status,
            jurisdiction: vec![],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: PublisherKey {
                key_id: efe_nano_sdk::KeyId::new(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: "x".into(),
                valid_from: SystemTime::now(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        }
    }

    #[tokio::test]
    async fn non_publisher_author_skips_check() {
        let gate = make_gate(crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY());
        let nano = sample_nano(Author::Efe);
        gate.assert_active(&nano).await.unwrap();
    }

    #[tokio::test]
    async fn builtin_publisher_resolved_without_registry() {
        let builtin = crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY();
        let builtin_id = builtin.publisher_id;
        let gate = make_gate(builtin);
        let nano = sample_nano(Author::Publisher {
            publisher_id: builtin_id,
            kyc_tier: KycTier::Tier3,
        });
        gate.assert_active(&nano).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_publisher_fails_closed() {
        let gate = make_gate(crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY());
        let nano = sample_nano(Author::Publisher {
            publisher_id: PublisherId::new(),
            kyc_tier: KycTier::Tier2,
        });
        let result = gate.assert_active(&nano).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn revoked_publisher_fails_closed_via_cache() {
        let id = PublisherId::new();
        let cache = Arc::new(PublisherIdentityCache::default());
        cache.put(id, fake_identity(id, PublisherStatus::Revoked));
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredPublisherStatusGate::new(
            Arc::new(InMemoryPublisherRegistry::new()),
            cache,
            metrics.clone(),
            Duration::from_millis(500),
            crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY(),
        );
        let nano = sample_nano(Author::Publisher {
            publisher_id: id,
            kyc_tier: KycTier::Tier2,
        });
        let result = gate.assert_active(&nano).await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("revoked"));
        assert_eq!(metrics.snapshot().status.revoked, 1);
    }

    #[tokio::test]
    async fn suspended_publisher_fails_closed_via_cache() {
        let id = PublisherId::new();
        let cache = Arc::new(PublisherIdentityCache::default());
        cache.put(
            id,
            fake_identity(
                id,
                PublisherStatus::Suspended {
                    reason: "investigation".into(),
                    suspended_at: SystemTime::now(),
                },
            ),
        );
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredPublisherStatusGate::new(
            Arc::new(InMemoryPublisherRegistry::new()),
            cache,
            metrics.clone(),
            Duration::from_millis(500),
            crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY(),
        );
        let nano = sample_nano(Author::Publisher {
            publisher_id: id,
            kyc_tier: KycTier::Tier2,
        });
        let result = gate.assert_active(&nano).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("suspended"));
        assert_eq!(metrics.snapshot().status.suspended, 1);
    }

    #[tokio::test]
    async fn active_publisher_passes_through_cache() {
        let id = PublisherId::new();
        let cache = Arc::new(PublisherIdentityCache::default());
        cache.put(id, fake_identity(id, PublisherStatus::Active));
        let metrics = Arc::new(N24ComplianceMetrics::new());
        let gate = WiredPublisherStatusGate::new(
            Arc::new(InMemoryPublisherRegistry::new()),
            cache,
            metrics.clone(),
            Duration::from_millis(500),
            crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY(),
        );
        let nano = sample_nano(Author::Publisher {
            publisher_id: id,
            kyc_tier: KycTier::Tier2,
        });
        gate.assert_active(&nano).await.unwrap();
        assert_eq!(metrics.snapshot().status.ok, 1);
    }
}
