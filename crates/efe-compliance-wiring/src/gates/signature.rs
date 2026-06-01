// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + G2 + I4 + I7** : `WiredSignatureGate` —
//! the only module that drives `Ed25519Verifier::verify` from
//! inside the swarm dispatch path.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use efe_nano_sdk::{Author, NanoAgent, PublisherIdentity, PublisherKey, PublisherSignature};
use efe_nano_swarm_adapter::SignatureGate;
use efe_publisher_identity::registry::{PublisherRegistry, RegistryError};
use efe_publisher_identity::verifier::SignatureVerifier;

use crate::cache::PublisherIdentityCache;
use crate::canonical::nano_to_canonical_body;
use crate::error::WiringError;
use crate::grace::GraceWindowResolver;
use crate::metrics::N24ComplianceMetrics;

/// **N24 + G2** : concrete `SignatureGate` impl bound to a
/// `(PublisherRegistry, SignatureVerifier)` pair.
pub struct WiredSignatureGate {
    registry: Arc<dyn PublisherRegistry>,
    verifier: Arc<dyn SignatureVerifier>,
    cache: Arc<PublisherIdentityCache>,
    grace: Arc<dyn GraceWindowResolver>,
    metrics: Arc<N24ComplianceMetrics>,
    lookup_timeout: Duration,
    builtin_identity: PublisherIdentity,
    builtin_tenant_id: Option<uuid::Uuid>,
}

impl std::fmt::Debug for WiredSignatureGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiredSignatureGate")
            .field("lookup_timeout", &self.lookup_timeout)
            .field(
                "builtin_publisher_id",
                &self.builtin_identity.publisher_id,
            )
            .finish_non_exhaustive()
    }
}

impl WiredSignatureGate {
    /// **N24** : construct the gate.  Callers normally do not
    /// build this directly — they go through
    /// `WiredComplianceRunner::new`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<dyn PublisherRegistry>,
        verifier: Arc<dyn SignatureVerifier>,
        cache: Arc<PublisherIdentityCache>,
        grace: Arc<dyn GraceWindowResolver>,
        metrics: Arc<N24ComplianceMetrics>,
        lookup_timeout: Duration,
        builtin_identity: PublisherIdentity,
        builtin_tenant_id: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            registry,
            verifier,
            cache,
            grace,
            metrics,
            lookup_timeout,
            builtin_identity,
            builtin_tenant_id,
        }
    }

    /// **N24** : resolve a `PublisherIdentity` via the cache
    /// (hit) or the registry (miss).  The registry call is
    /// wrapped in a `tokio::time::timeout` (I6 / F10).
    ///
    /// Special-case : if the publisher id matches the baked-in
    /// EFE built-in publisher (ADR-9), return the baked identity
    /// directly without consulting the registry — this resolves
    /// the chicken-and-egg of cold boot (the registry might be
    /// empty when the gateway first signs its own catalogue).
    async fn resolve_identity(
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

    /// **N24 + I4** : find the key matching the signature.
    /// Exact `key_id` match wins ; fall back to
    /// `key_active_at(signed_at)`.
    fn find_key<'a>(
        identity: &'a PublisherIdentity,
        signature: &PublisherSignature,
    ) -> Option<&'a PublisherKey> {
        // Exact key_id match wins (I4).
        if identity.active_key.key_id == signature.key_id {
            return Some(&identity.active_key);
        }
        if let Some(k) = identity
            .historical_keys
            .iter()
            .find(|k| k.key_id == signature.key_id)
        {
            return Some(k);
        }
        // Fall through to was_active_at.
        identity.key_active_at(signature.signed_at)
    }

    /// **N24 + G14 + ADR-10** : grace-window check.  Returns
    /// `true` if the (optional) tenant id is currently within
    /// the grace window AND the nano is unsigned.
    ///
    /// Sprint 16 ships this as a private helper for Sprint 17
    /// adapter-level wiring (the gate itself is only called
    /// when a signature exists).
    #[allow(dead_code)]
    async fn allow_grace_skip(&self, signed: bool) -> bool {
        if signed {
            return false;
        }
        let Some(tenant_id) = self.builtin_tenant_id else {
            return false;
        };
        self.grace
            .in_grace_window(tenant_id, std::time::SystemTime::now())
            .await
    }
}

#[async_trait]
impl SignatureGate for WiredSignatureGate {
    async fn verify(
        &self,
        nano: &NanoAgent,
        signature: &PublisherSignature,
    ) -> Result<(), String> {
        // 1. Author-vs-signer match (G8 / I17).
        if let Author::Publisher {
            publisher_id: author_id,
            ..
        } = &nano.author
        {
            if *author_id != signature.publisher_id {
                self.metrics.signature_verified_total("author_mismatch");
                return Err(WiringError::AuthorMismatch {
                    nano_pub_id: *author_id,
                    sig_pub_id: signature.publisher_id,
                }
                .to_string());
            }
        }

        // 2. Resolve publisher (cache → registry).
        let identity = match self.resolve_identity(signature.publisher_id).await {
            Ok(id) => id,
            Err(WiringError::PublisherNotFound(p)) => {
                self.metrics.signature_verified_total("publisher_not_found");
                return Err(WiringError::PublisherNotFound(p).to_string());
            }
            Err(e) => {
                self.metrics.signature_verified_total("other");
                return Err(e.to_string());
            }
        };

        // 3. Find the key matching `signature.key_id` (I4).
        let key = match Self::find_key(&identity, signature) {
            Some(k) => k,
            None => {
                self.metrics.signature_verified_total("key_not_found");
                return Err(WiringError::KeyNotFound {
                    key_id: signature.key_id,
                    signed_at: signature.signed_at,
                }
                .to_string());
            }
        };

        // 4. Reconstruct canonical body (I2 / A1).
        let canonical = nano_to_canonical_body(nano, signature);
        let bytes = canonical.canonical_bytes();

        // 5. Cryptographic verify (I3).
        match self
            .verifier
            .verify(signature, &bytes, &key.public_key_pem)
            .await
        {
            Ok(()) => {
                self.metrics.signature_verified_total("ok");
                Ok(())
            }
            Err(e) => {
                self.metrics.signature_verified_total("verify_failed");
                Err(WiringError::SignatureFailed(e.to_string()).to_string())
            }
        }
    }
}

// **Sprint 17 NOTE** : the grace short-circuit for unsigned
// catalogue entries belongs at the *adapter* layer, not the
// signature gate (the gate is only called when a signature
// exists).  Sprint 16 keeps `allow_grace_skip` as a private
// helper for future use ; the adapter's dispatch will consult
// `policy.publisher_status_gate.in_grace_window(tenant_id)`
// before deciding whether to enforce signature presence.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::PublisherIdentityCache;
    use crate::grace::DenyAllGrace;
    use crate::metrics::N24ComplianceMetrics;
    use chrono::Utc;
    use efe_nano_sdk::{
        Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, KeyId, KycTier,
        NanoAgentId, NanoAgentVersion, PublisherId, ResourceLimits, Sha256Hash,
        SignatureAlgorithm, StabilityTier,
    };
    use efe_publisher_identity::registry::InMemoryPublisherRegistry;
    use efe_publisher_identity::verifier::{Ed25519Signer, Ed25519Verifier};
    use std::time::SystemTime;

    fn sample_nano(author: Author, signature: Option<PublisherSignature>) -> NanoAgent {
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
            signature,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    fn sample_sig(
        publisher_id: PublisherId,
        key_id: KeyId,
        bytes: Vec<u8>,
        canonical_hash: Sha256Hash,
        signed_at: SystemTime,
    ) -> PublisherSignature {
        PublisherSignature {
            publisher_id,
            key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            signature: bytes,
            signed_at,
            canonical_hash,
        }
    }

    fn make_gate(
        registry: Arc<dyn PublisherRegistry>,
        builtin: PublisherIdentity,
    ) -> WiredSignatureGate {
        WiredSignatureGate::new(
            registry,
            Arc::new(Ed25519Verifier::new()),
            Arc::new(PublisherIdentityCache::default()),
            Arc::new(DenyAllGrace),
            Arc::new(N24ComplianceMetrics::new()),
            Duration::from_millis(500),
            builtin,
            None,
        )
    }

    #[tokio::test]
    async fn author_publisher_id_mismatch_rejected() {
        let registry = Arc::new(InMemoryPublisherRegistry::new());
        let builtin = crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY();
        let gate = make_gate(registry, builtin);
        let signer = Ed25519Signer::generate();
        let body = b"x";
        let _sig_bytes = signer.sign(body);
        // Author id != signature publisher id.
        let nano = sample_nano(
            Author::Publisher {
                publisher_id: PublisherId::new(),
                kyc_tier: KycTier::Tier1,
            },
            None,
        );
        let sig = sample_sig(
            PublisherId::new(),
            KeyId::new(),
            vec![0u8; 64],
            Sha256Hash::from_bytes([0u8; 32]),
            SystemTime::now(),
        );
        let result = gate.verify(&nano, &sig).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[tokio::test]
    async fn builtin_publisher_short_circuit_resolves_identity() {
        let registry = Arc::new(InMemoryPublisherRegistry::new());
        let builtin = crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY();
        let builtin_id = builtin.publisher_id;
        let gate = make_gate(registry, builtin);
        let identity = gate.resolve_identity(builtin_id).await.unwrap();
        assert_eq!(identity.publisher_id, builtin_id);
    }

    #[tokio::test]
    async fn unknown_publisher_returns_not_found() {
        let registry = Arc::new(InMemoryPublisherRegistry::new());
        let builtin = crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY();
        let gate = make_gate(registry, builtin);
        let unknown = PublisherId::new();
        let result = gate.resolve_identity(unknown).await;
        match result {
            Err(WiringError::PublisherNotFound(id)) => {
                assert_eq!(id, unknown);
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn signed_builtin_catalogue_verifies_through_gate() {
        let registry = Arc::new(InMemoryPublisherRegistry::new());
        let builtin = crate::builtin::EFE_BUILTIN_PUBLISHER_IDENTITY();
        let gate = make_gate(registry, builtin);
        let signed = crate::builtin::sign_catalogue_with_builtin_publisher(
            efe_nano_preanalysis::default_catalogue(),
            None,
        );
        for (nano, _) in &signed {
            let sig = nano.signature.as_ref().unwrap();
            gate.verify(nano, sig)
                .await
                .expect("built-in catalogue signature MUST verify via WiredSignatureGate");
        }
    }

    #[test]
    fn find_key_prefers_exact_key_id_match() {
        use chrono::TimeZone;
        let active_key = PublisherKey {
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "pem_active".into(),
            valid_from: SystemTime::now(),
            valid_until: None,
        };
        let historical_key_id = KeyId::new();
        let historical_key = PublisherKey {
            key_id: historical_key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "pem_historical".into(),
            valid_from: SystemTime::UNIX_EPOCH,
            valid_until: Some(SystemTime::now()),
        };
        let identity = PublisherIdentity {
            publisher_id: PublisherId::new(),
            display_name: "x".into(),
            kyc_tier: KycTier::Tier2,
            status: efe_nano_sdk::PublisherStatus::Active,
            jurisdiction: vec![],
            registered_at: SystemTime::UNIX_EPOCH,
            last_sanctions_check_at: None,
            active_key,
            historical_keys: vec![historical_key],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        };
        // Signature references the HISTORICAL key id — find_key
        // must return it (not the active one) even though both
        // are technically active at signed_at.
        let _ = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0);
        let sig = sample_sig(
            PublisherId::new(),
            historical_key_id,
            vec![],
            Sha256Hash::from_bytes([0u8; 32]),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        );
        let found = WiredSignatureGate::find_key(&identity, &sig).unwrap();
        assert_eq!(found.public_key_pem, "pem_historical");
    }
}
