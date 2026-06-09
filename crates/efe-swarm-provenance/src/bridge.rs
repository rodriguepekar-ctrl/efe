// SPDX-License-Identifier: UNLICENSED
//! # `bridge` — le **pont Mode A ↔ Mode C** (`llm_provenance_id` → N17).
//!
//! L'essaim mêle deux granularités de provenance :
//! - **Mode A** (Rust pur) : un [`crate::record::AgentCallRecord`] **autonome** —
//!   aucun appel LLM, donc aucun lien N17 attendu.
//! - **Mode C** (LLM) : l'agent fait un appel `LlmClient` qui produit un
//!   `efe_nano_sdk::LlmProvenanceRecord` (N17) identifié par un [`ProvenanceId`].
//!   Le record d'essaim **référence** ce N17 via son `llm_provenance_id` →
//!   **une seule lignée, deux granularités**.
//!
//! Ce module fournit :
//! 1. [`LlmProvenanceView`] — la **vue minimale** d'un record N17 (les seuls champs
//!    utiles au pont) + l'adaptateur [`LlmProvenanceView::from_record`] vers le **vrai**
//!    type N17 (`efe_nano_sdk::LlmProvenanceRecord`). Découple le pont du store N17.
//! 2. [`LlmResolver`] — suit un `ProvenanceId` → la vue (un store/query N17 réel
//!    s'adapte derrière ce trait ; [`MapResolver`] pour tests / cache).
//! 3. [`classify_link`] / [`verify_bridge`] — **vérifie** la cohérence des liens :
//!    le N17 existe, **même tenant**, **même swarm**. Détecte les lacunes (Mode C sans
//!    lien) et les pointeurs pendants.
//!
//! La **pose** du lien se fait à la capture (`ProvenancedAgent::with_llm_link`, ou
//! `CallCapture::with_llm_provenance`) : un agent Mode C surface son `ProvenanceId`.

use std::collections::HashMap;

use efe_nano_sdk::{AgentId, Blake3Hash, LlmProvenanceRecord, ProvenanceId, TenantId};
use uuid::Uuid;

use crate::record::{AgentCallRecord, AgentKind};

// ─────────────────────────────────────────────────────────────────────────────
//  Vue minimale d'un record N17 + adaptateur vers le vrai type
// ─────────────────────────────────────────────────────────────────────────────

/// Les seuls champs d'un record N17 dont le pont a besoin pour vérifier un lien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProvenanceView {
    pub provenance_id: ProvenanceId,
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub output_hash: Option<Blake3Hash>,
}

impl LlmProvenanceView {
    /// Projette un **vrai** `LlmProvenanceRecord` (N17) sur la vue. C'est l'adaptateur
    /// qui relie ce pont au type réel d'`efe-llm-provenance`.
    pub fn from_record(r: &LlmProvenanceRecord) -> Self {
        Self {
            provenance_id: r.provenance_id,
            tenant_id: r.tenant_id,
            swarm_request_id: r.swarm_request_id,
            output_hash: r.output_hash,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Résolveur
// ─────────────────────────────────────────────────────────────────────────────

/// Suit un `ProvenanceId` → la vue N17. Un store/query N17 réel (async) s'adapte en
/// pré-chargeant un [`MapResolver`], ou en implémentant ce trait directement.
pub trait LlmResolver {
    fn resolve(&self, id: ProvenanceId) -> Option<LlmProvenanceView>;
}

/// Résolveur en mémoire : un cache de vues (tests, ou snapshot d'un store N17).
#[derive(Debug, Default, Clone)]
pub struct MapResolver {
    map: HashMap<Uuid, LlmProvenanceView>,
}

impl MapResolver {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, view: LlmProvenanceView) -> &mut Self {
        self.map.insert(view.provenance_id.as_uuid(), view);
        self
    }
    /// Pré-charge depuis de **vrais** records N17 (le pont vers `efe-llm-provenance`).
    pub fn from_records<'a, I>(records: I) -> Self
    where
        I: IntoIterator<Item = &'a LlmProvenanceRecord>,
    {
        let mut m = Self::new();
        for r in records {
            m.insert(LlmProvenanceView::from_record(r));
        }
        m
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl LlmResolver for MapResolver {
    fn resolve(&self, id: ProvenanceId) -> Option<LlmProvenanceView> {
        self.map.get(&id.as_uuid()).cloned()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Classification + vérification des liens
// ─────────────────────────────────────────────────────────────────────────────

/// Statut du lien d'un enregistrement d'essaim vers N17.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// Agent **Mode A** : aucun lien N17 attendu (état sain).
    ModeA,
    /// **Mode C sans** `llm_provenance_id` : **lacune** (l'agent n'a pas exposé son id N17).
    Unlinked,
    /// Lien **présent et cohérent** : le N17 existe, même tenant, même swarm.
    Consistent,
    /// `llm_provenance_id` présent mais **introuvable** (pointeur pendant).
    Missing,
    /// Le record N17 appartient à un **autre tenant** (anomalie d'isolation).
    TenantMismatch,
    /// Le record N17 a été produit dans un **autre swarm**.
    SwarmMismatch,
    /// **Mode A portant un lien** (anomalie : un agent pur ne fait pas d'appel LLM).
    UnexpectedLinkOnModeA,
}

impl LinkStatus {
    /// État sain (Mode A sans lien, ou Mode C lié cohérent).
    pub fn is_ok(&self) -> bool {
        matches!(self, LinkStatus::ModeA | LinkStatus::Consistent)
    }
    /// Rupture réelle (pointeur pendant / incohérence d'isolation).
    pub fn is_broken(&self) -> bool {
        matches!(
            self,
            LinkStatus::Missing
                | LinkStatus::TenantMismatch
                | LinkStatus::SwarmMismatch
                | LinkStatus::UnexpectedLinkOnModeA
        )
    }
}

/// Classe le lien N17 d'un enregistrement d'essaim.
pub fn classify_link<R: LlmResolver>(record: &AgentCallRecord, resolver: &R) -> LinkStatus {
    match record.agent_kind {
        AgentKind::ModeA => {
            if record.llm_provenance_id.is_some() {
                LinkStatus::UnexpectedLinkOnModeA
            } else {
                LinkStatus::ModeA
            }
        }
        AgentKind::ModeC => match record.llm_provenance_id {
            None => LinkStatus::Unlinked,
            Some(id) => match resolver.resolve(id) {
                None => LinkStatus::Missing,
                Some(view) => {
                    if view.tenant_id != record.tenant_id {
                        LinkStatus::TenantMismatch
                    } else if view.swarm_request_id != record.swarm_id.as_uuid() {
                        LinkStatus::SwarmMismatch
                    } else {
                        LinkStatus::Consistent
                    }
                }
            },
        },
    }
}

/// Suit le lien : la vue N17 d'un enregistrement (None si Mode A / non lié / introuvable).
pub fn resolve_llm<R: LlmResolver>(record: &AgentCallRecord, resolver: &R) -> Option<LlmProvenanceView> {
    record.llm_provenance_id.and_then(|id| resolver.resolve(id))
}

/// Un lien classé (pour le rapport).
#[derive(Debug, Clone)]
pub struct BridgeLink {
    pub record_id: Uuid,
    pub agent_id: AgentId,
    pub status: LinkStatus,
    pub llm_provenance_id: Option<ProvenanceId>,
}

/// Bilan du pont sur un ensemble d'enregistrements.
#[derive(Debug, Clone, Default)]
pub struct BridgeReport {
    /// Agents Mode A (aucun lien attendu).
    pub mode_a: usize,
    /// Mode C liés et cohérents.
    pub consistent: usize,
    /// Mode C **sans** lien (lacunes).
    pub unlinked: usize,
    /// Liens rompus (pendant / incohérent).
    pub broken: usize,
    /// Détail par enregistrement.
    pub links: Vec<BridgeLink>,
}

impl BridgeReport {
    pub fn total(&self) -> usize {
        self.links.len()
    }
    /// Tous les Mode C sont liés et cohérents (aucune lacune, aucune rupture).
    pub fn all_mode_c_consistent(&self) -> bool {
        self.unlinked == 0 && self.broken == 0
    }
    /// Itère les liens rompus (forensique).
    pub fn broken_links(&self) -> impl Iterator<Item = &BridgeLink> {
        self.links.iter().filter(|l| l.status.is_broken())
    }
}

/// **Vérifie le pont** : classe chaque enregistrement (Mode A / lié cohérent / lacune /
/// rompu) → [`BridgeReport`]. Ne dépend QUE des enregistrements + un résolveur N17.
pub fn verify_bridge<R: LlmResolver>(records: &[AgentCallRecord], resolver: &R) -> BridgeReport {
    let mut rep = BridgeReport::default();
    for r in records {
        let status = classify_link(r, resolver);
        match status {
            LinkStatus::ModeA => rep.mode_a += 1,
            LinkStatus::Consistent => rep.consistent += 1,
            LinkStatus::Unlinked => rep.unlinked += 1,
            _ => rep.broken += 1,
        }
        rep.links.push(BridgeLink {
            record_id: r.record_id,
            agent_id: r.agent_id.clone(),
            status,
            llm_provenance_id: r.llm_provenance_id,
        });
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{AgentCallDraft, AgentRole};
    use efe_nano_sdk::{
        Blake3Hash, Completeness, CostAccounting, FinishReason, ModelIdentity, NanoAgentId,
        NanoAgentVersion, PiiStrategy, RegionCode, RetryOutcome, SamplingParams, SwarmId,
        TokenUsage,
    };
    use std::time::SystemTime;
    use uuid::Uuid;

    fn swarm_uuid() -> Uuid {
        Uuid::from_u128(0x2222)
    }
    fn swarm() -> SwarmId {
        SwarmId::from_uuid(swarm_uuid())
    }

    /// Un **vrai** record N17 (réplique de `sample_record`) avec id/tenant/swarm donnés.
    fn n17(prov: ProvenanceId, tenant: TenantId, swarm_request_id: Uuid) -> LlmProvenanceRecord {
        LlmProvenanceRecord {
            provenance_id: prov,
            trace_id: Uuid::new_v4(),
            swarm_request_id,
            op_id: "m50.classify_topic.v1".into(),
            tenant_id: tenant,
            agent_id: NanoAgentId::new("judge_llm").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: ModelIdentity::hosted("local", "llama", "v1"),
            model_hash_verified: false,
            prompt_template_id: "judge-v1".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::from_bytes([1u8; 32]),
            system_prompt_hash: Blake3Hash::from_bytes([2u8; 32]),
            user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([4u8; 32]),
            sampling: SamplingParams { temperature: 0.0, top_p: None, max_tokens: 32, seed: Some(7), stop_sequences: vec![] },
            token_usage: TokenUsage { client_estimated_input: 10, client_estimated_output: 2, provider_reported_input: None, provider_reported_output: None, is_estimated: true },
            cost_accounting: CostAccounting { cost_cents: 0.0, prices_table_version: 1, provider_quoted_cost_cents: None, overflow_capped: false },
            provider_request_id: None,
            provider_response_time_ms: 5,
            provider_finish_reason: FinishReason::Stop,
            provider_region: Some(RegionCode::UsEast1),
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: std::collections::HashMap::new(),
            output_hash: Some(Blake3Hash::from_bytes([5u8; 32])),
            completeness: Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::Strict,
            redaction_token_set: vec![],
            replay_of: None,
            cache_hit: None,
            recorded_at: SystemTime::now(),
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
        }
    }

    /// Enregistrement d'essaim (scellé avec un hmac factice — le pont ne le vérifie pas).
    fn rec(kind: AgentKind, tenant: TenantId, sw: SwarmId, llm: Option<ProvenanceId>) -> AgentCallRecord {
        let mut d = AgentCallDraft::new(
            sw, tenant, AgentId::new("judge_000").unwrap(), AgentRole::Judge, kind,
            Blake3Hash::from_bytes([9u8; 32]), Blake3Hash::from_bytes([8u8; 32]),
        );
        if let Some(id) = llm {
            d = d.with_llm_provenance(id);
        }
        d.seal(0, None, [0u8; 32])
    }

    #[test]
    fn from_record_projects_real_n17_fields() {
        let prov = ProvenanceId::new();
        let r = n17(prov, TenantId::system(), swarm_uuid());
        let v = LlmProvenanceView::from_record(&r);
        assert_eq!(v.provenance_id, prov);
        assert_eq!(v.tenant_id, TenantId::system());
        assert_eq!(v.swarm_request_id, swarm_uuid());
        assert_eq!(v.output_hash, Some(Blake3Hash::from_bytes([5u8; 32])));
    }

    #[test]
    fn mode_c_with_consistent_link_is_consistent() {
        let prov = ProvenanceId::new();
        let resolver = MapResolver::from_records([&n17(prov, TenantId::system(), swarm_uuid())]);
        let record = rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(prov));
        assert_eq!(classify_link(&record, &resolver), LinkStatus::Consistent);
        // on peut SUIVRE le lien jusqu'au N17.
        assert!(resolve_llm(&record, &resolver).is_some());
    }

    #[test]
    fn mode_a_expects_no_link() {
        let resolver = MapResolver::new();
        let record = rec(AgentKind::ModeA, TenantId::system(), swarm(), None);
        assert_eq!(classify_link(&record, &resolver), LinkStatus::ModeA);
    }

    #[test]
    fn mode_c_without_link_is_a_gap() {
        let resolver = MapResolver::new();
        let record = rec(AgentKind::ModeC, TenantId::system(), swarm(), None);
        assert_eq!(classify_link(&record, &resolver), LinkStatus::Unlinked);
    }

    #[test]
    fn dangling_pointer_is_missing() {
        let resolver = MapResolver::new(); // vide → l'id ne résout pas
        let record = rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(ProvenanceId::new()));
        assert_eq!(classify_link(&record, &resolver), LinkStatus::Missing);
    }

    #[test]
    fn tenant_mismatch_is_detected() {
        let prov = ProvenanceId::new();
        let other = TenantId::new();
        let resolver = MapResolver::from_records([&n17(prov, other, swarm_uuid())]);
        let record = rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(prov));
        assert_eq!(classify_link(&record, &resolver), LinkStatus::TenantMismatch);
    }

    #[test]
    fn swarm_mismatch_is_detected() {
        let prov = ProvenanceId::new();
        let resolver = MapResolver::from_records([&n17(prov, TenantId::system(), Uuid::from_u128(0x9999))]);
        let record = rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(prov));
        assert_eq!(classify_link(&record, &resolver), LinkStatus::SwarmMismatch);
    }

    #[test]
    fn mode_a_with_unexpected_link_is_flagged() {
        let resolver = MapResolver::new();
        let record = rec(AgentKind::ModeA, TenantId::system(), swarm(), Some(ProvenanceId::new()));
        assert_eq!(classify_link(&record, &resolver), LinkStatus::UnexpectedLinkOnModeA);
    }

    #[test]
    fn verify_bridge_aggregates_a_mixed_run() {
        let prov = ProvenanceId::new();
        let resolver = MapResolver::from_records([&n17(prov, TenantId::system(), swarm_uuid())]);
        let records = vec![
            rec(AgentKind::ModeA, TenantId::system(), swarm(), None),       // Mode A
            rec(AgentKind::ModeA, TenantId::system(), swarm(), None),       // Mode A
            rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(prov)), // lié cohérent
            rec(AgentKind::ModeC, TenantId::system(), swarm(), None),       // lacune
            rec(AgentKind::ModeC, TenantId::system(), swarm(), Some(ProvenanceId::new())), // pendant
        ];
        let rep = verify_bridge(&records, &resolver);
        assert_eq!(rep.mode_a, 2);
        assert_eq!(rep.consistent, 1);
        assert_eq!(rep.unlinked, 1);
        assert_eq!(rep.broken, 1);
        assert_eq!(rep.total(), 5);
        assert!(!rep.all_mode_c_consistent(), "il y a une lacune et une rupture");
        assert_eq!(rep.broken_links().count(), 1);
    }
}
