// SPDX-License-Identifier: UNLICENSED
//! **Pont N17 par-appel** — chaque appel d'un agent **Mode C** (LLM local) produit un
//! `LlmProvenanceRecord` **forensique** (modèle · params · hashs prompt/réponse · latence),
//! et l'`AgentCallRecord` d'essaim le **référence** via `llm_provenance_id`
//! (`efe_swarm_provenance::bridge`). **Une seule lignée, deux granularités.**
//!
//! ## Le seam (sans réécrire les agents)
//! Le backend ne connaît pas le contexte d'essaim (swarm/tenant/agent) ; le `Meter` ne voit
//! pas l'appel LLM brut (modèle/prompt). On les relie par un **task-local** :
//! 1. Le `Meter` enveloppe l'appel d'agent dans [`scoped`] (pose un puits par-appel).
//! 2. Le backend, à chaque `complete()`, **pousse** une [`PendingLlmCall`] (forensique LLM)
//!    via [`push_pending`] — `no-op` hors d'un scope (Mode A, ou backend non instrumenté).
//! 3. Le `Meter`, l'appel terminé, **draine** le puits, **assemble** les
//!    `LlmProvenanceRecord` (ajoute le contexte d'essaim) dans un [`LlmProvenanceCollector`],
//!    et **lie** l'`AgentCallRecord` au premier `ProvenanceId`.
//!
//! Concurrence : un puits **par appel d'agent** (task-local scopé) → pas de course même
//! quand K générateurs tournent en parallèle. Le `curl` du backend est en `spawn_blocking`,
//! mais le `push_pending` s'exécute **après** son retour, sur la tâche d'origine → task-local visible.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use efe_nano_sdk::{
    AgentId, Blake3Hash, Completeness, CostAccounting, FinishReason, LlmProvenanceRecord,
    ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, RetryOutcome,
    SamplingParams as N17Sampling, SwarmId, TenantId, TokenUsage,
};
use efe_swarm_provenance::{blake3_bytes, AgentRole};

/// Forensique d'**un** appel LLM, produite par le backend (sans contexte d'essaim).
pub(crate) struct PendingLlmCall {
    pub engine: String, // "ollama" / "openai_local"
    pub model: String,
    pub temperature: f64,
    pub max_tokens: u32,
    pub seed: u64,
    pub prompt: String,
    pub response: Option<String>,
    pub latency_ms: u64,
}

tokio::task_local! {
    static PENDING: Arc<Mutex<Vec<PendingLlmCall>>>;
}

/// Pousse la forensique d'un appel LLM dans le puits courant. **No-op** hors scope.
pub(crate) fn push_pending(call: PendingLlmCall) {
    let _ = PENDING.try_with(|p| {
        if let Ok(mut v) = p.lock() {
            v.push(call);
        }
    });
}

/// Exécute `fut` dans un scope qui collecte les appels LLM dans `sink`.
pub(crate) fn scoped<F>(sink: Arc<Mutex<Vec<PendingLlmCall>>>, fut: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    PENDING.scope(sink, fut)
}

fn op_id_for(role: AgentRole) -> String {
    format!("swarm.{}", format!("{role:?}").to_lowercase())
}

/// Estimation grossière de tokens (≈ 4 chars/token) — `is_estimated = true` (local, pas de
/// compteur provider). Honnête : on ne prétend pas connaître le tokenizer.
fn estimate_tokens(s: &str) -> u32 {
    (s.chars().count() / 4).max(1) as u32
}

/// Collecteur des `LlmProvenanceRecord` d'un run (par-appel). Assemble depuis une
/// [`PendingLlmCall`] + le contexte d'essaim, stocke, renvoie le `ProvenanceId`.
#[derive(Default)]
pub struct LlmProvenanceCollector {
    records: Mutex<Vec<LlmProvenanceRecord>>,
}

impl LlmProvenanceCollector {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Assemble + stocke un record N17 forensique ; renvoie son `ProvenanceId` (le lien).
    pub(crate) fn assemble(
        &self,
        swarm_id: SwarmId,
        tenant: TenantId,
        agent_id: &AgentId,
        role: AgentRole,
        call: PendingLlmCall,
    ) -> ProvenanceId {
        let provenance_id = ProvenanceId::new();
        let ok = call.response.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
        let user_prompt_hash = blake3_bytes(call.prompt.as_bytes());
        let output_hash = call.response.as_ref().map(|r| blake3_bytes(r.as_bytes()));

        let record = LlmProvenanceRecord {
            // identité
            provenance_id,
            trace_id: ProvenanceId::new().as_uuid(), // uuid v4 sans dépendre du crate `uuid`
            swarm_request_id: swarm_id.as_uuid(),
            op_id: op_id_for(role),
            tenant_id: tenant,
            agent_id: NanoAgentId::new(agent_id.as_str()).unwrap_or_else(|_| NanoAgentId::new("swarm_agent").unwrap()),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            // modèle (self-host local : provider = moteur, version = "local")
            model_identity: ModelIdentity::hosted(&call.engine, &call.model, "local"),
            model_hash_verified: false,
            // provenance du prompt (pas de gabarit : user = final ; système/inputs vides)
            prompt_template_id: "swarm.selfhost".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::ZERO,
            system_prompt_hash: Blake3Hash::ZERO,
            user_prompt_hash,
            final_prompt_hash: user_prompt_hash,
            // échantillonnage (seed exposé)
            sampling: N17Sampling {
                temperature: call.temperature as f32,
                top_p: None,
                max_tokens: call.max_tokens,
                seed: Some(call.seed),
                stop_sequences: vec![],
            },
            // tokens (estimés, local) + coût nul (compute local)
            token_usage: TokenUsage {
                client_estimated_input: estimate_tokens(&call.prompt),
                client_estimated_output: call.response.as_deref().map(estimate_tokens).unwrap_or(0),
                provider_reported_input: None,
                provider_reported_output: None,
                is_estimated: true,
            },
            cost_accounting: CostAccounting { cost_cents: 0.0, prices_table_version: 0, provider_quoted_cost_cents: None, overflow_capped: false },
            // métadonnées provider (local)
            provider_request_id: None,
            provider_response_time_ms: call.latency_ms.min(u32::MAX as u64) as u32,
            provider_finish_reason: if ok { FinishReason::Stop } else { FinishReason::Error },
            provider_region: None,
            provider_metadata: std::collections::HashMap::new(),
            // sortie + complétude
            output_hash,
            completeness: if ok { Completeness::Complete } else { Completeness::Errored { error_kind: "empty_response".into() } },
            safety_flags: vec![],
            tool_calls: vec![],
            // PII (local : aucune redaction par défaut)
            pii_strategy_applied: PiiStrategy::None,
            redaction_token_set: vec![],
            // N18/N19 transverses : pas de publisher/KYC/gold pour un agent self-host local.
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            // transverses
            replay_of: None,
            cache_hit: None,
            recorded_at: SystemTime::now(),
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
        };
        self.records.lock().expect("n17 collector lock").push(record);
        provenance_id
    }

    /// Copie des records assemblés (forensique par-appel).
    pub fn records(&self) -> Vec<LlmProvenanceRecord> {
        self.records.lock().map(|v| v.clone()).unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.records.lock().map(|v| v.len()).unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_provenance::{
        classify_link, verify_bridge, AgentCallDraft, LinkStatus, MapResolver,
    };

    fn call(prompt: &str, resp: Option<&str>) -> PendingLlmCall {
        PendingLlmCall {
            engine: "ollama".into(),
            model: "gemma3:12b".into(),
            temperature: 0.2,
            max_tokens: 16,
            seed: 42,
            prompt: prompt.into(),
            response: resp.map(String::from),
            latency_ms: 1234,
        }
    }

    #[test]
    fn assemble_builds_forensic_record_with_real_hashes() {
        let c = LlmProvenanceCollector::default();
        let aid = AgentId::new("judge_000").unwrap();
        let id = c.assemble(SwarmId::new(), TenantId::system(), &aid, AgentRole::Judge, call("classify this", Some("legal")));
        let recs = c.records();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.provenance_id, id);
        assert_eq!(r.model_identity.provider, "ollama");
        assert_eq!(r.model_identity.model_id, "gemma3:12b");
        assert_eq!(r.op_id, "swarm.judge");
        assert_eq!(r.sampling.seed, Some(42));
        // hashs RÉELS (blake3 du prompt / de la réponse).
        assert_eq!(r.user_prompt_hash, blake3_bytes(b"classify this"));
        assert_eq!(r.output_hash, Some(blake3_bytes(b"legal")));
        assert!(matches!(r.completeness, Completeness::Complete));
    }

    #[test]
    fn empty_response_is_errored() {
        let c = LlmProvenanceCollector::default();
        let aid = AgentId::new("generator").unwrap();
        c.assemble(SwarmId::new(), TenantId::system(), &aid, AgentRole::Generator, call("x", None));
        let r = &c.records()[0];
        assert!(matches!(r.completeness, Completeness::Errored { .. }));
        assert_eq!(r.provider_finish_reason, FinishReason::Error);
        assert!(r.output_hash.is_none());
    }

    #[test]
    fn assembled_record_resolves_a_consistent_bridge_link() {
        // Le record N17 assemblé est résolvable par le pont → lien Cohérent.
        let c = LlmProvenanceCollector::default();
        let swarm = SwarmId::new();
        let tenant = TenantId::system();
        let aid = AgentId::new("generator").unwrap();
        let id = c.assemble(swarm, tenant, &aid, AgentRole::Generator, call("p", Some("legal")));

        // un AgentCallRecord Mode C lié au ProvenanceId.
        let rec = AgentCallDraft::new(
            swarm, tenant, aid, AgentRole::Generator, efe_swarm_provenance::AgentKind::ModeC,
            blake3_bytes(b"in"), blake3_bytes(b"out"),
        )
        .with_llm_provenance(id)
        .seal(0, None, [0u8; 32]);

        let resolver = MapResolver::from_records(c.records().iter());
        assert_eq!(classify_link(&rec, &resolver), LinkStatus::Consistent);
        let report = verify_bridge(std::slice::from_ref(&rec), &resolver);
        assert_eq!(report.consistent, 1);
        assert!(report.all_mode_c_consistent());
    }
}
