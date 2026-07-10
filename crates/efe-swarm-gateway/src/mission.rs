// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 — M1 : capture de provenance par appel LLM.**
//!
//! Chaque appel d'un provider frontière (Claude/Gemini/GPT) doit produire un
//! [`LlmProvenanceRecord`] forensique **scellable** (modèle, hachés blake3 prompt/réponse,
//! coût, latence, **attestation de canal R4**). On y parvient **sans toucher aux missions**
//! (`efe-multi-llm`) ni au fan-out (`efe-coordinator`) : un **décorateur**
//! [`ProvenanceProvider`] enveloppe chaque `Arc<dyn LlmProvider>` du registre et, à chaque
//! `call()`, assemble un record qu'il **pousse** dans un puits par-mission via un **scope
//! task-local** — miroir exact de `efe-swarm-sdk::n17` (le pont N17 de l'essaim).
//!
//! ## Le seam
//! 1. Le handler de mission (M3) enveloppe l'exécution dans [`scoped`] (pose un
//!    [`MissionScope`] : tenant, id de mission, puits).
//! 2. Le décorateur, à chaque appel, lit le scope, assemble le record et le **pousse**
//!    (`no-op` hors scope — un appel émis hors mission ne pollue aucun puits).
//! 3. Le handler, la mission finie, **draine** le puits ([`MissionProvenanceSink::drain`]) ;
//!    **M2** en construira le graphe Mode-C (depuis la topologie déclarée) et scellera les racines.
//!
//! ## Souveraineté (correctifs #2/#3/#4 de l'architecture)
//! - **#2 endpoint observé** : la localité (R2 loopback vs R4 distant) est décidée sur l'URL
//!   **réelle** exposée par `LlmProvider::endpoint()`, classée par le guard partagé
//!   `provenance_capture::is_local_endpoint` — jamais inférée, jamais une 3e copie du guard.
//! - **#4 invariant R4** : un endpoint non-local ⇒ `R4RemoteChannel { keyed: true }` **toujours**
//!   posé (jamais le défaut `LegacyUnspecified`, qui mentirait par omission sur l'egress).
//! - **#3 échecs capturés** : un appel en échec produit AUSSI un record (`Errored`) — les octets
//!   sont partis, le reçu doit le dire ; le décorateur enregistre avant de propager l'`Err`.
//!
//! **Streaming** : `stream()` n'est pas surchargé — l'impl **par défaut** du trait route par
//! `self.call()`, donc un appel streaming est lui aussi **bufferisé + capturé** ici. L'instrumentation
//! d'un vrai flux incrémental (token-par-token) est différée (le MVP des missions M3 est sync-only).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use efe_coordinator::{CoordResult, LlmCall, LlmProvider, LlmResponse};
use efe_nano_sdk::{
    Blake3Hash, Completeness, ComputeAttestation, CostAccounting, EndpointIdentity, FinishReason,
    LlmProvenanceRecord, ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId,
    RetryOutcome, SamplingParams, TenantId, TokenUsage,
};
use efe_swarm_provenance::blake3_bytes;
use uuid::Uuid;

/// Puits collectant les records de provenance d'une mission (un par appel LLM).
#[derive(Default)]
pub struct MissionProvenanceSink {
    records: Mutex<Vec<LlmProvenanceRecord>>,
}

impl MissionProvenanceSink {
    /// Puits neuf, partagé (`Arc`) entre le scope task-local et le handler qui le drainera.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Pousse un record (best-effort : un mutex empoisonné n'est jamais une panique en cascade).
    fn push(&self, rec: LlmProvenanceRecord) {
        if let Ok(mut v) = self.records.lock() {
            v.push(rec);
        }
    }

    /// **Draine** tous les records collectés (vide le puits). Appelé par le handler en fin de
    /// mission ; **M2** consommera ce `Vec` pour bâtir le graphe Mode-C + sceller les racines.
    pub fn drain(&self) -> Vec<LlmProvenanceRecord> {
        self.records.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }

    /// Nombre de records collectés (sans drainer) — sert aux gardiens de test.
    pub fn len(&self) -> usize {
        self.records.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// `true` si aucun appel n'a encore été capturé.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Contexte d'une mission, porté par le scope task-local : tenant + id de mission + puits.
#[derive(Clone)]
pub struct MissionScope {
    /// Le tenant propriétaire (scelle le record ; isolation).
    pub tenant_id: TenantId,
    /// L'identité de la mission (devient `swarm_request_id` du record — le lien Mode-C de M2).
    pub mission_id: Uuid,
    /// Le puits où les appels décorés déposent leur provenance.
    pub sink: Arc<MissionProvenanceSink>,
}

tokio::task_local! {
    static MISSION: MissionScope;
}

/// Exécute `fut` dans un scope de mission : les appels LLM des providers **décorés** y
/// collectent leur provenance dans `scope.sink`. Miroir de `efe-swarm-sdk::n17::scoped`.
/// Hors de ce scope, la capture est un **no-op** (aucune fuite, aucun puits pollué).
pub async fn scoped<F>(scope: MissionScope, fut: F) -> F::Output
where
    F: std::future::Future,
{
    MISSION.scope(scope, fut).await
}

/// Estimation grossière de tokens (≈ 4 chars/token) — honnête : `is_estimated = true`.
fn estimate_tokens(s: &str) -> u32 {
    (s.chars().count() / 4).max(1) as u32
}

/// **Décorateur de provenance** : enveloppe un provider et scelle chaque appel en record N17.
/// Implémente `LlmProvider`, donc s'enregistre dans le registre du coordinator **à la place**
/// du provider nu — les missions l'appellent sans le savoir.
pub struct ProvenanceProvider {
    inner: Arc<dyn LlmProvider>,
}

impl ProvenanceProvider {
    /// Enveloppe `inner`. `coordinator_from_env` enregistre `wrap(provider)` au lieu du provider nu.
    pub fn wrap(inner: Arc<dyn LlmProvider>) -> Arc<Self> {
        Arc::new(Self { inner })
    }

    /// Assemble un record forensique depuis la requête + le résultat + le scope courant.
    fn build_record(
        &self,
        scope: &MissionScope,
        prompt: &str,
        temperature: f32,
        max_tokens: u32,
        result: &CoordResult<LlmResponse>,
    ) -> LlmProvenanceRecord {
        let provider_id = self.inner.id();

        // #2 endpoint OBSERVÉ + #4 invariant R4 : classe la localité sur l'URL réelle du provider.
        let compute_attestation = match self.inner.endpoint() {
            Some(url) if provenance_capture::is_local_endpoint(&url) => {
                ComputeAttestation::R2LocalWitnessed
            }
            Some(url) => ComputeAttestation::R4RemoteChannel {
                // `pin_fingerprint` vide : le pinning TLS n'est pas encore appliqué (différé) ;
                // `keyed: true` = un credential a transité — jamais enregistré (I-RC5).
                endpoint: EndpointIdentity::new(url, String::new()),
                keyed: true,
            },
            // Aucun canal observable (mock/in-mémoire) : on n'affirme RIEN sur la localité
            // (défaut le plus faible — pas de mensonge par omission, cf. LegacyUnspecified).
            None => ComputeAttestation::LegacyUnspecified,
        };
        let region = match &compute_attestation {
            ComputeAttestation::R3RemoteCosigned { endpoint, .. }
            | ComputeAttestation::R4RemoteChannel { endpoint, .. } => endpoint.host.clone(),
            _ => "local".to_string(),
        };

        // Un échec (ou une réponse vide) = record `Errored`, output_hash absent.
        let (response_text, ok, latency_ms, tokens_out, cost_microcents) = match result {
            Ok(r) => (
                Some(r.text.clone()),
                !r.text.is_empty(),
                r.latency_ms,
                r.tokens_generated,
                r.cost_microcents_resolved(),
            ),
            Err(_) => (None, false, 0, 0, 0),
        };

        let user_prompt_hash = blake3_bytes(prompt.as_bytes());
        let output_hash = response_text.as_ref().map(|t| blake3_bytes(t.as_bytes()));

        LlmProvenanceRecord {
            provenance_id: ProvenanceId::new(),
            trace_id: ProvenanceId::new().as_uuid(),
            swarm_request_id: scope.mission_id,
            op_id: format!("couche2.frontier.{provider_id}"),
            tenant_id: scope.tenant_id,
            agent_id: NanoAgentId::new(provider_id.as_str())
                .unwrap_or_else(|_| NanoAgentId::new("llm_frontier").unwrap()),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: ModelIdentity::hosted(provider_id.as_str(), provider_id.as_str(), region.as_str()),
            model_hash_verified: false,
            prompt_template_id: "couche2.frontier".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::ZERO,
            system_prompt_hash: Blake3Hash::ZERO,
            user_prompt_hash,
            final_prompt_hash: user_prompt_hash,
            sampling: SamplingParams {
                temperature,
                top_p: None,
                max_tokens,
                seed: None,
                stop_sequences: vec![],
            },
            token_usage: TokenUsage {
                client_estimated_input: estimate_tokens(prompt),
                client_estimated_output: response_text.as_deref().map(estimate_tokens).unwrap_or(0),
                provider_reported_input: None,
                provider_reported_output: if ok { Some(tokens_out) } else { None },
                is_estimated: true,
            },
            cost_accounting: CostAccounting {
                cost_cents: cost_microcents as f64 / 1000.0,
                prices_table_version: 0,
                provider_quoted_cost_cents: None,
                overflow_capped: false,
            },
            provider_request_id: None,
            provider_response_time_ms: latency_ms.min(u32::MAX as u64) as u32,
            provider_finish_reason: if ok { FinishReason::Stop } else { FinishReason::Error },
            provider_region: None,
            provider_metadata: HashMap::new(),
            output_hash,
            completeness: if ok {
                Completeness::Complete
            } else {
                Completeness::Errored { error_kind: "provider_error_or_empty".into() }
            },
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::None,
            redaction_token_set: vec![],
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            replay_of: None,
            cache_hit: None,
            recorded_at: SystemTime::now(),
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
            compute_attestation,
        }
    }
}

#[async_trait]
impl LlmProvider for ProvenanceProvider {
    fn id(&self) -> String {
        self.inner.id()
    }

    fn endpoint(&self) -> Option<String> {
        self.inner.endpoint()
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        // Capture les entrées AVANT le `move` de `req` dans l'appel interne.
        let prompt = req.prompt.clone();
        let temperature = req.temperature;
        let max_tokens = req.max_tokens;

        let result = self.inner.call(req).await;

        // #3 : on scelle un record MÊME en échec (les octets sont partis). `no-op` hors scope.
        let _ = MISSION.try_with(|scope| {
            let rec = self.build_record(scope, &prompt, temperature, max_tokens, &result);
            scope.sink.push(rec);
        });

        result
    }

    async fn health(&self) -> CoordResult<bool> {
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_coordinator::error::CoordError;

    /// Provider factice : réponse fixe OU erreur, endpoint configurable (pour exercer R2/R4/None).
    struct FakeProvider {
        id: String,
        endpoint: Option<String>,
        fail: bool,
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn id(&self) -> String {
            self.id.clone()
        }
        fn endpoint(&self) -> Option<String> {
            self.endpoint.clone()
        }
        async fn call(&self, _req: LlmCall) -> CoordResult<LlmResponse> {
            if self.fail {
                return Err(CoordError::LlmProvider { provider: self.id.clone(), reason: "boom".into() });
            }
            Ok(LlmResponse {
                text: "the answer".into(),
                tokens_generated: 3,
                cost_cents: 1,
                cost_microcents: 45,
                latency_ms: 12,
                provider: self.id.clone(),
                self_quality: 1.0,
            })
        }
    }

    fn scope() -> MissionScope {
        MissionScope { tenant_id: TenantId::system(), mission_id: Uuid::from_u128(7), sink: MissionProvenanceSink::new() }
    }

    #[tokio::test]
    async fn remote_endpoint_yields_r4_record_with_real_hashes() {
        let p = ProvenanceProvider::wrap(Arc::new(FakeProvider {
            id: "claude_x".into(),
            endpoint: Some("https://api.anthropic.com/v1/messages".into()),
            fail: false,
        }));
        let s = scope();
        let sink = s.sink.clone();
        let out = scoped(s, async { p.call(LlmCall::simple("classify this", 16)).await }).await;
        assert!(out.is_ok());
        let recs = sink.drain();
        assert_eq!(recs.len(), 1, "un record par appel");
        let r = &recs[0];
        // #4 : endpoint distant OBSERVÉ ⇒ R4 keyed (jamais LegacyUnspecified).
        assert!(matches!(r.compute_attestation, ComputeAttestation::R4RemoteChannel { keyed: true, .. }));
        assert!(r.compute_attestation.is_remote(), "any_remote = true");
        // hachés RÉELS.
        assert_eq!(r.user_prompt_hash, blake3_bytes(b"classify this"));
        assert_eq!(r.output_hash, Some(blake3_bytes(b"the answer")));
        assert!(matches!(r.completeness, Completeness::Complete));
        assert_eq!(r.swarm_request_id, Uuid::from_u128(7), "lié à la mission");
    }

    #[tokio::test]
    async fn failed_call_is_still_recorded() {
        // #3 : un appel en échec produit un record `Errored` (les octets sont partis).
        let p = ProvenanceProvider::wrap(Arc::new(FakeProvider {
            id: "gpt_x".into(),
            endpoint: Some("https://api.openai.com/v1/chat/completions".into()),
            fail: true,
        }));
        let s = scope();
        let sink = s.sink.clone();
        let out = scoped(s, async { p.call(LlmCall::simple("p", 8)).await }).await;
        assert!(out.is_err(), "l'erreur est propagée");
        let recs = sink.drain();
        assert_eq!(recs.len(), 1, "l'échec figure quand même au reçu");
        assert!(matches!(recs[0].completeness, Completeness::Errored { .. }));
        assert!(recs[0].output_hash.is_none());
        assert_eq!(recs[0].provider_finish_reason, FinishReason::Error);
    }

    #[tokio::test]
    async fn no_endpoint_is_legacy_unspecified_not_a_lie() {
        // Sans canal observable, on n'affirme RIEN (défaut le plus faible), pas de faux R2/R4.
        let p = ProvenanceProvider::wrap(Arc::new(FakeProvider { id: "mock".into(), endpoint: None, fail: false }));
        let s = scope();
        let sink = s.sink.clone();
        scoped(s, async { let _ = p.call(LlmCall::simple("x", 4)).await; }).await;
        let recs = sink.drain();
        assert!(matches!(recs[0].compute_attestation, ComputeAttestation::LegacyUnspecified));
        assert!(!recs[0].compute_attestation.is_remote());
    }

    #[tokio::test]
    async fn call_outside_scope_captures_nothing() {
        // Hors mission : aucun puits, la capture est un no-op (pas de panic, pas de fuite).
        let p = ProvenanceProvider::wrap(Arc::new(FakeProvider {
            id: "claude_x".into(),
            endpoint: Some("https://api.anthropic.com".into()),
            fail: false,
        }));
        let out = p.call(LlmCall::simple("x", 4)).await;
        assert!(out.is_ok(), "l'appel marche toujours hors scope");
    }
}
