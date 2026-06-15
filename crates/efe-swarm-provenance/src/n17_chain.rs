// SPDX-License-Identifier: UNLICENSED
//! # `n17_chain` — la forensique LLM **durable, scellée, et re-prouvable hors ligne** (ADR-016).
//!
//! Le pont N17 (`bridge`) prouve, à l'instant du run, « quel modèle a produit cette
//! sortie, avec quel prompt, quelle graine ». Mais la forensique N17 (un
//! [`LlmProvenanceRecord`] par appel LLM) **s'évaporait** à la fin du run : la passerelle
//! la jetait en persistant, et le `llm_provenance_id` de chaque agent Mode C devenait un
//! pointeur pendant. Ce module ferme la faille.
//!
//! ## Deux chaînes, un seul lien sécurisé
//! La chaîne d'essaim (`SwarmChain`) scelle déjà les [`AgentCallRecord`] — dont le
//! **lien** `llm_provenance_id`, commis dans leur payload. Ce module scelle la **cible**
//! du lien — le contenu forensique — dans une **chaîne HMAC parallèle** (`n17_root`),
//! avec la **même clé tenant** (HKDF, ADR-007) :
//! ```text
//! feuille_i = SHA-256(canonical_bytes(record_i))           (le contenu, déterministe)
//! hmac_0 = HMAC(genesis ‖ feuille_0, clé) ; hmac_1 = HMAC(hmac_0 ‖ feuille_1, clé) ; …
//!                                                  └── tip = n17_root
//! ```
//! Le lien est scellé là (chaîne d'essaim), sa cible est scellée ici (chaîne N17), et
//! [`verify_bridge`] les recoud. **Tamper-evident de bout en bout, hors ligne.**
//!
//! ## Format de record d'essaim GELÉ
//! Une chaîne *parallèle* (et non une fusion) parce que l'`AgentCallRecord` ne porte que
//! l'`id` N17, pas son hash de contenu — y ajouter le hash changerait le format gelé et
//! invaliderait les runs déjà persistés. Parité exacte avec ADR-014/015 : zéro champ
//! ajouté à l'atome d'essaim.
//!
//! ## Honnêteté (caveat per-run, §4.1 de l'architecture)
//! Un record N17 porte des champs intrinsèquement per-run (`recorded_at`, `trace_id` v4) ;
//! `n17_root` est donc, comme `provenance_root`, une empreinte **par-run**. Ce qui est
//! re-prouvable est que les records persistés **recomposent exactement** le `n17_root`
//! stocké — la tamper-evidence, pas une racine universelle.
//!
//! ## Localisation (best-effort, honnête)
//! On persiste aussi les **feuilles** (les SHA-256 de contenu, ordre canonique). À la
//! vérification : une feuille recalculée qui diffère de la feuille stockée **localise** le
//! record muté ; si les feuilles ont elles aussi été éditées pour coller, le repli HMAC
//! (clé tenant, non forgeable) casse au niveau chaîne — détecté sans localisation fine.

use efe_audit_core::chain_hmac;
use efe_nano_sdk::{LlmProvenanceRecord, ProvenanceId};

use crate::hash::sha256;
use crate::record::hex32;

/// Le hash de contenu **déterministe** d'un record N17 : SHA-256 de sa forme JSON
/// canonique (clés triées, sans espace — insensible à l'ordre de sérialisation).
fn leaf_hash(record: &LlmProvenanceRecord) -> [u8; 32] {
    // `LlmProvenanceRecord` est `Serialize` ; sa projection en `Value` est totale.
    let value = serde_json::to_value(record).expect("LlmProvenanceRecord est Serialize");
    sha256(&crate::canon::canonical_bytes(&value))
}

/// Ordre canonique des records N17 : par `provenance_id` (déterministe et stable à la
/// relecture — chaque record porte son id). PAS l'ordre d'assemblage (qui dépend de
/// l'ordonnancement concurrent des K agents — la leçon d'ADR-007).
fn canonical_order(records: &[LlmProvenanceRecord]) -> Vec<&LlmProvenanceRecord> {
    let mut v: Vec<&LlmProvenanceRecord> = records.iter().collect();
    v.sort_by_key(|r| r.provenance_id.as_uuid());
    v
}

/// Le résultat d'un scellement N17 : les feuilles (ordre canonique) + la racine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedN17 {
    /// SHA-256 de contenu par record, en **ordre canonique** (par provenance_id).
    pub leaf_hashes: Vec<[u8; 32]>,
    /// `n17_root` : le HMAC replié sur les feuilles ordonnées (clé tenant). `None` si vide.
    pub n17_root: Option<[u8; 32]>,
}

impl SealedN17 {
    /// `n17_root` en hex (64 car.) pour la persistance / le fil.
    pub fn root_hex(&self) -> Option<String> {
        self.n17_root.as_ref().map(hex32)
    }
    /// Les feuilles en hex (pour la persistance de localisation).
    pub fn leaves_hex(&self) -> Vec<String> {
        self.leaf_hashes.iter().map(hex32).collect()
    }
}

/// **Scelle** un ensemble de records N17 en chaîne HMAC propre (clé tenant déjà dérivée).
/// Ordre canonique, feuille = SHA-256 de contenu, repli HMAC → `n17_root`. Pur,
/// déterministe (à set de records fixé).
pub fn seal_n17(records: &[LlmProvenanceRecord], key: &[u8]) -> SealedN17 {
    let ordered = canonical_order(records);
    let mut leaf_hashes = Vec::with_capacity(ordered.len());
    let mut tip: Option<[u8; 32]> = None;
    for r in ordered {
        let leaf = leaf_hash(r);
        tip = Some(chain_hmac(tip.as_ref(), &leaf, key));
        leaf_hashes.push(leaf);
    }
    SealedN17 { leaf_hashes, n17_root: tip }
}

/// Pourquoi la chaîne N17 casse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum N17Break {
    /// Le contenu d'un record a été muté (sa feuille recalculée ≠ la feuille stockée).
    RecordContentMutated,
    /// Le repli HMAC ne correspond plus à `n17_root` (feuilles/racine forgées sans la clé).
    ChainRootMismatch,
    /// Le nombre de records relus ≠ le nombre de feuilles scellées (ajout/suppression).
    CountMismatch,
}

/// Verdict de re-preuve de la chaîne N17.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct N17ChainVerdict {
    /// Chaîne N17 intègre (toutes les feuilles correspondent ET le repli == `n17_root`).
    pub intact: bool,
    /// Nombre de records N17 vérifiés.
    pub records: usize,
    /// `n17_root` recalculé depuis les records persistés.
    pub root: Option<[u8; 32]>,
    /// Le record fautif **localisé** (mutation de contenu), si déterminable.
    pub first_break: Option<ProvenanceId>,
    /// La nature de la rupture (`None` si intègre).
    pub reason: Option<N17Break>,
}

impl N17ChainVerdict {
    fn ok(records: usize, root: Option<[u8; 32]>) -> Self {
        Self { intact: true, records, root, first_break: None, reason: None }
    }
    pub fn root_hex(&self) -> Option<String> {
        self.root.as_ref().map(hex32)
    }
}

/// **Re-prouve la chaîne N17** depuis les SEULS records persistés + les feuilles scellées +
/// `n17_root` + la clé tenant. Localise un record muté quand les feuilles n'ont pas été
/// touchées ; sinon casse au niveau chaîne (repli HMAC non forgeable). Pur, hors ligne.
pub fn verify_n17_chain(
    records: &[LlmProvenanceRecord],
    sealed_leaves: &[[u8; 32]],
    n17_root: Option<[u8; 32]>,
    key: &[u8],
) -> N17ChainVerdict {
    let ordered = canonical_order(records);

    // (a) cardinalité : un record ajouté/supprimé après scellement.
    if ordered.len() != sealed_leaves.len() {
        return N17ChainVerdict {
            intact: false,
            records: ordered.len(),
            root: None,
            first_break: ordered.first().map(|r| r.provenance_id),
            reason: Some(N17Break::CountMismatch),
        };
    }

    // (b) contenu : chaque feuille recalculée doit égaler la feuille scellée → localise.
    for (r, sealed) in ordered.iter().zip(sealed_leaves.iter()) {
        if &leaf_hash(r) != sealed {
            return N17ChainVerdict {
                intact: false,
                records: ordered.len(),
                root: None,
                first_break: Some(r.provenance_id),
                reason: Some(N17Break::RecordContentMutated),
            };
        }
    }

    // (c) chaîne : rejoue le HMAC sur les feuilles scellées → compare à n17_root.
    let mut tip: Option<[u8; 32]> = None;
    for leaf in sealed_leaves {
        tip = Some(chain_hmac(tip.as_ref(), leaf, key));
    }
    if tip != n17_root {
        return N17ChainVerdict {
            intact: false,
            records: ordered.len(),
            root: tip,
            first_break: None, // forge de feuilles/racine : pas de localisation fine
            reason: Some(N17Break::ChainRootMismatch),
        };
    }

    N17ChainVerdict::ok(ordered.len(), tip)
}

// ─────────────────────────────────────────────────────────────────────────────
//  verify_run_offline — la re-preuve COMPLÈTE, pure (essaim + N17 + pont)
// ─────────────────────────────────────────────────────────────────────────────

use crate::bridge::{verify_bridge, BridgeReport, MapResolver};
use crate::record::AgentCallRecord;
use crate::verify::{verify_swarm, SwarmProvenanceReport};

/// Le verdict d'une re-preuve complète hors ligne.
#[derive(Debug, Clone)]
pub struct OfflineVerdict {
    /// Chaîne d'essaim + contenu + DAG (`verify_swarm`).
    pub swarm: SwarmProvenanceReport,
    /// Chaîne N17 (la forensique se recompose-t-elle ?).
    pub n17_chain: N17ChainVerdict,
    /// Pont Mode A ↔ Mode C (chaque Mode C résout-il en cohérent ?).
    pub bridge: BridgeReport,
    /// Tout est re-prouvé : essaim OK, chaîne N17 intacte, pont sans lacune ni rupture.
    pub fully_proven: bool,
}

/// **Re-prouve TOUT** depuis les seuls records persistés (essaim + N17) + feuilles +
/// `n17_root` + clé tenant. Aucune async, aucun réseau, aucun provider — un tiers, des
/// jours plus tard : la définition même du moat, étendue à la forensique LLM (ADR-016).
pub fn verify_run_offline(
    swarm_records: &[AgentCallRecord],
    n17_records: &[LlmProvenanceRecord],
    n17_leaves: &[[u8; 32]],
    n17_root: Option<[u8; 32]>,
    key: &[u8],
) -> OfflineVerdict {
    let swarm = verify_swarm(swarm_records, key);
    let n17_chain = verify_n17_chain(n17_records, n17_leaves, n17_root, key);
    // Le seam déjà prévu : un résolveur pré-chargé depuis les records N17 persistés.
    let resolver = MapResolver::from_records(n17_records.iter());
    let bridge = verify_bridge(swarm_records, &resolver);
    let fully_proven = swarm.ok() && n17_chain.intact && bridge.all_mode_c_consistent();
    OfflineVerdict { swarm, n17_chain, bridge, fully_proven }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        AgentId, Blake3Hash, Completeness, CostAccounting, FinishReason, ModelIdentity,
        NanoAgentId, NanoAgentVersion, PiiStrategy, RetryOutcome, SamplingParams, SwarmId,
        TenantId, TokenUsage,
    };
    use std::time::SystemTime;
    use uuid::Uuid;

    fn key() -> Vec<u8> {
        efe_audit_core::HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }
    fn swarm_uuid() -> Uuid {
        Uuid::from_u128(0x2222)
    }

    /// Un record N17 déterministe (id fixe → ordre canonique reproductible en test).
    fn n17(prov_u128: u128, tenant: TenantId, swarm: Uuid, model: &str) -> LlmProvenanceRecord {
        LlmProvenanceRecord {
            provenance_id: ProvenanceId::from_uuid(Uuid::from_u128(prov_u128)),
            trace_id: Uuid::from_u128(0xAA00 + prov_u128),
            swarm_request_id: swarm,
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
            recorded_at: SystemTime::UNIX_EPOCH,
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
        }
    }

    fn three() -> Vec<LlmProvenanceRecord> {
        vec![
            n17(1, TenantId::system(), swarm_uuid(), "gemma3:12b"),
            n17(2, TenantId::system(), swarm_uuid(), "gemma3:12b"),
            n17(3, TenantId::system(), swarm_uuid(), "gemma3:12b"),
        ]
    }

    #[test]
    fn seal_then_verify_is_intact() {
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        assert_eq!(sealed.leaf_hashes.len(), 3);
        assert!(sealed.n17_root.is_some());
        let v = verify_n17_chain(&recs, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(v.intact, "chaîne N17 saine : {v:?}");
        assert_eq!(v.records, 3);
        assert_eq!(v.root, sealed.n17_root, "root recalculé == scellé");
        assert!(v.first_break.is_none());
    }

    #[test]
    fn canonical_order_is_deterministic_regardless_of_input_order() {
        // mêmes records, ordres d'ENTRÉE différents → même scellement (tri par provenance_id).
        let a = three();
        let mut b = three();
        b.reverse();
        assert_eq!(seal_n17(&a, &key()), seal_n17(&b, &key()), "ordre d'entrée neutre");
    }

    #[test]
    fn mutated_n17_field_breaks_the_chain_and_localizes() {
        // I-PN2 : muter un champ forensique (le modèle) casse la chaîne ET localise.
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        let mut tampered = recs.clone();
        tampered[1].model_identity = ModelIdentity::hosted("ollama", "MENTEUR:70b", "local");
        let v = verify_n17_chain(&tampered, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(N17Break::RecordContentMutated));
        assert_eq!(v.first_break, Some(recs[1].provenance_id), "le record muté est localisé");
    }

    #[test]
    fn mutated_seed_is_also_caught() {
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        let mut tampered = recs.clone();
        tampered[2].sampling.seed = Some(999); // graine substituée
        let v = verify_n17_chain(&tampered, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact);
        assert_eq!(v.first_break, Some(recs[2].provenance_id));
    }

    #[test]
    fn deleted_record_breaks_root() {
        // (gardien §9.1 « deleted_record_breaks_root ») — mécanisme : CountMismatch. Retirer
        // un record N17 fait que le compte relu ≠ scellé : la racine ne peut plus recomposer.
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        let v = verify_n17_chain(&recs[..2], &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(N17Break::CountMismatch));
    }

    #[test]
    fn forged_leaves_and_root_break_at_chain_level() {
        // un attaquant mute un record ET réécrit sa feuille pour coller — mais ne peut
        // pas recomputer le HMAC (pas la clé) → casse au niveau chaîne.
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        let mut tampered = recs.clone();
        tampered[0].user_prompt_hash = Blake3Hash::from_bytes([0xEE; 32]);
        // l'attaquant recompute la feuille du record muté (il sait hasher), mais garde
        // l'ancien root (il ne peut pas le re-HMAC sans la clé).
        let mut leaves = sealed.leaf_hashes.clone();
        let ordered_idx = {
            // position canonique du record muté
            let mut ids: Vec<Uuid> = recs.iter().map(|r| r.provenance_id.as_uuid()).collect();
            ids.sort();
            ids.iter().position(|u| *u == recs[0].provenance_id.as_uuid()).unwrap()
        };
        leaves[ordered_idx] = leaf_hash(&tampered[0]); // feuille forgée pour coller au contenu muté
        let v = verify_n17_chain(&tampered, &leaves, sealed.n17_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(N17Break::ChainRootMismatch), "le repli HMAC non forgeable casse");
    }

    #[test]
    fn wrong_key_breaks_the_chain() {
        let recs = three();
        let sealed = seal_n17(&recs, &key());
        let v = verify_n17_chain(&recs, &sealed.leaf_hashes, sealed.n17_root, &[0u8; 32]);
        assert!(!v.intact);
        assert_eq!(v.reason, Some(N17Break::ChainRootMismatch));
    }

    #[test]
    fn empty_n17_seals_and_verifies_vacuously() {
        let sealed = seal_n17(&[], &key());
        assert_eq!(sealed.n17_root, None);
        assert!(sealed.leaf_hashes.is_empty());
        let v = verify_n17_chain(&[], &[], None, &key());
        assert!(v.intact, "0 forensique (Mode A) = vide trivialement intègre");
    }

    #[test]
    fn offline_verify_reproves_swarm_n17_and_bridge() {
        use crate::record::{AgentCallDraft, AgentKind, AgentRole};
        // un run Mode C minimal : 1 agent lié à 1 N17, scellé en chaîne d'essaim + N17.
        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(swarm_uuid());
        let n17_recs = vec![n17(1, tenant, swarm_uuid(), "gemma3:12b")];
        let sealed_n17 = seal_n17(&n17_recs, &key());

        // l'AgentCallRecord Mode C référence le N17 (le lien), scellé dans une chaîne d'essaim.
        let draft = AgentCallDraft::new(
            swarm, tenant, AgentId::new("generator").unwrap(), AgentRole::Generator,
            AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_llm_provenance(n17_recs[0].provenance_id);
        let mut chain = crate::chain::SwarmChain::new(tenant, swarm, key());
        let swarm_recs = chain.append_all(std::iter::once(draft));

        let v = verify_run_offline(&swarm_recs, &n17_recs, &sealed_n17.leaf_hashes, sealed_n17.n17_root, &key());
        assert!(v.fully_proven, "tout re-prouvé hors ligne : {:?}", (v.swarm.ok(), v.n17_chain.intact, v.bridge.all_mode_c_consistent()));
        assert!(v.swarm.ok());
        assert!(v.n17_chain.intact);
        assert_eq!(v.bridge.consistent, 1, "le Mode C résout en cohérent depuis le store N17");
    }

    #[test]
    fn offline_verify_catches_a_dangling_link_after_n17_loss() {
        use crate::record::{AgentCallDraft, AgentKind, AgentRole};
        // le lien Mode C est là, mais la forensique N17 a été perdue (store vide) →
        // verify_bridge → Missing → pas fully_proven (le pointeur pendant est ATTRAPÉ).
        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(swarm_uuid());
        let draft = AgentCallDraft::new(
            swarm, tenant, AgentId::new("generator").unwrap(), AgentRole::Generator,
            AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_llm_provenance(ProvenanceId::from_uuid(Uuid::from_u128(0xDEAD)));
        let mut chain = crate::chain::SwarmChain::new(tenant, swarm, key());
        let swarm_recs = chain.append_all(std::iter::once(draft));

        let v = verify_run_offline(&swarm_recs, &[], &[], None, &key());
        assert!(v.swarm.ok(), "la chaîne d'essaim, elle, est intègre");
        assert!(!v.fully_proven, "mais le lien pendant empêche la preuve complète");
        assert_eq!(v.bridge.broken, 1, "Missing détecté");
    }
}
