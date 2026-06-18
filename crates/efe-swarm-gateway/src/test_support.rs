// SPDX-License-Identifier: UNLICENSED
//! Utilitaires de test partagés (ADR-016) : un run Mode C **synthétique** (sans Ollama),
//! pour éprouver la persistance + la re-preuve forensique. Réutilisé par les tests de
//! `lib.rs` (passerelle HTTP, in-memory) et de `store_pg.rs` (round-trip Postgres non vide).
//! Une seule définition → pas de littéral `LlmProvenanceRecord` dupliqué.

use serde_json::json;
use uuid::Uuid;

use efe_swarm_sdk::HmacKeySource;

use crate::StoredRun;

/// Fabrique un run Mode C synthétique : un appel LLM lié à sa forensique N17, scellé en chaîne
/// d'essaim. Renvoie (records d'essaim, `StoredRun` prêt à persister). Le tenant = système
/// (UUID 00…001), pour s'aligner sur l'appelant par défaut côté HTTP.
pub(crate) fn synthetic_mode_c_run(model: &str) -> (Vec<efe_swarm_provenance::AgentCallRecord>, StoredRun) {
    use efe_nano_sdk::{
        Blake3Hash, Completeness, CostAccounting, FinishReason, LlmProvenanceRecord, ModelIdentity,
        NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, RetryOutcome, SamplingParams,
        SwarmId, TenantId, TokenUsage,
    };
    use efe_swarm_provenance::{seal_n17, AgentCallDraft, AgentKind, AgentRole, SwarmChain};

    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x5EED));
    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let prov = ProvenanceId::from_uuid(Uuid::from_u128(0xC0FFEE));

    // la forensique N17 d'un appel LLM.
    let llm = LlmProvenanceRecord {
        provenance_id: prov,
        trace_id: Uuid::from_u128(0xBEEF),
        swarm_request_id: swarm.as_uuid(),
        op_id: "swarm.generator".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("generator").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("ollama", model, "local"),
        model_hash_verified: false,
        prompt_template_id: "swarm.selfhost".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
        final_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
        sampling: SamplingParams { temperature: 0.2, top_p: None, max_tokens: 16, seed: Some(7), stop_sequences: vec![] },
        token_usage: TokenUsage { client_estimated_input: 5, client_estimated_output: 2, provider_reported_input: None, provider_reported_output: None, is_estimated: true },
        cost_accounting: CostAccounting { cost_cents: 0.0, prices_table_version: 0, provider_quoted_cost_cents: None, overflow_capped: false },
        provider_request_id: None,
        provider_response_time_ms: 5,
        provider_finish_reason: FinishReason::Stop,
        provider_region: None,
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
        provider_metadata: std::collections::HashMap::new(),
        output_hash: Some(Blake3Hash::from_bytes([5u8; 32])),
        completeness: Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: std::time::SystemTime::UNIX_EPOCH,
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
        compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
    };

    // l'AgentCallRecord Mode C qui RÉFÉRENCE la forensique (le lien), scellé en chaîne.
    let draft = AgentCallDraft::new(
        swarm, tenant, efe_nano_sdk::AgentId::new("generator").unwrap(), AgentRole::Generator,
        AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
    )
    .with_llm_provenance(prov);
    let mut chain = SwarmChain::new(tenant, swarm, key.clone());
    let records = chain.append_all(std::iter::once(draft));
    let root_hex = chain.root_hex();

    let sealed = seal_n17(std::slice::from_ref(&llm), &key);
    let stored = StoredRun {
        // Le tenant par défaut d'un appelant sans en-tête = `TenantId::system()` rendu en UUID
        // (00…001) ; on s'y aligne pour que `store.get` retrouve le run injecté, et pour que
        // `derive_key` recompose la MÊME clé (from_str(uuid) == system).
        tenant: tenant.to_string(),
        records: records.clone(),
        root: root_hex,
        llm_records: vec![llm],
        n17_leaves: sealed.leaves_hex(),
        n17_root: sealed.root_hex(),
        output: json!({ "value": "ok" }),
        topology: "T-consensus".into(),
        created_at: std::time::SystemTime::now(),
    };
    (records, stored)
}
