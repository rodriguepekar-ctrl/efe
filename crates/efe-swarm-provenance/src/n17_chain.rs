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

use std::collections::HashMap;

use efe_audit_core::chain_hmac;
use efe_nano_sdk::{ComputeAttestation, LlmProvenanceRecord, ProvenanceId};

use crate::cosign::{verify_cosignature, CosignDowngrade, NodeRegistry};
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

/// **RC-1** — le barreau de confiance de computation **le plus faible** tenu par le run, sur
/// tous ses records Mode C. Le vérifieur le rapporte **toujours**, à côté de `fully_proven`
/// (jamais replié dedans — exactement le patron de `traced_pct`) : c'est la forme opérationnelle
/// d'**I-RC8**. Un run propre contenant un record R4 se lit « transaction re-prouvée au barreau
/// R4 ; computation **non** témoignée », jamais comme une re-preuve de la computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldStrength {
    /// Rang du barreau le plus faible (0 = legacy/non-asserti … 4 = R1-déterministe).
    /// `None` ⇒ aucun record Mode C dans le run (rien à attester côté computation).
    pub weakest_rank: Option<u8>,
    /// Libellé du barreau le plus faible (forensique), p.ex. `"R4-DISTANT-CANAL"`.
    pub weakest_label: Option<String>,
    /// Au moins un record a quitté la boîte souveraine (barreau R3 ou R4).
    pub any_remote: bool,
}

impl HeldStrength {
    /// Le barreau le plus faible parmi les records Mode C (le « maillon faible » du run).
    pub fn from_records(records: &[LlmProvenanceRecord]) -> Self {
        let mut weakest: Option<&ComputeAttestation> = None;
        let mut any_remote = false;
        for r in records {
            let a = &r.compute_attestation;
            any_remote |= a.is_remote();
            weakest = match weakest {
                Some(w) if w.strength_rank() <= a.strength_rank() => Some(w),
                _ => Some(a),
            };
        }
        HeldStrength {
            weakest_rank: weakest.map(ComputeAttestation::strength_rank),
            weakest_label: weakest.map(|a| a.rung_label().to_string()),
            any_remote,
        }
    }
}

/// **RC-2/R3** — un écart revendiqué↔vérifié signalé par le vérifieur : un record qui REVENDIQUE
/// R3 mais dont la co-signature n'a pas pu être vérifiée (donc redescendu à R4). Jamais silencieux
/// (I-RC9). Le `node_id` et la `reason` rendent la dégradation auditable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosignAnomaly {
    /// Le record concerné (localisation).
    pub provenance_id: ProvenanceId,
    /// Le nœud revendiqué par la co-signature.
    pub node_id: String,
    /// Pourquoi la co-signature n'a pas été honorée (→ redescente R4).
    pub reason: CosignDowngrade,
}

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
    /// **Garantie de la couche TRANSACTION** — distincte du barreau de *computation* ci-dessous.
    pub fully_proven: bool,
    /// **RC-1 / I-RC8** : le barreau de computation le plus faible tenu par le run. Toujours
    /// rapporté ; jamais replié dans `fully_proven`. Voir [`HeldStrength`].
    pub held_strength: HeldStrength,
    /// **RC-2/R3 / I-RC9** : les revendications R3 non honorées (redescendues à R4), avec leur
    /// raison. Vide ⇒ aucune sur-prétention détectée. Jamais silencieux.
    pub cosign_anomalies: Vec<CosignAnomaly>,
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
    // Sans registre de nœuds : aucune clé pinnée ⇒ toute revendication R3 est invérifiable et
    // redescend à R4 (I-RC9, défaut honnête). Pour HONORER R3, passer un NodeRegistry (ci-dessous).
    verify_run_offline_with_nodes(swarm_records, n17_records, n17_leaves, n17_root, key, &NodeRegistry::empty())
}

/// **RC-2/R3** — `verify_run_offline` conscient des nœuds : vérifie les co-signatures R3 contre le
/// `node_registry` (clés publiques pinnées + statut). Une revendication R3 **honorée** tient le
/// record à R3 ; une revendication **invérifiable** le redescend à R4 et l'ajoute aux
/// `cosign_anomalies` (I-RC9 : aucune élévation sans preuve, jamais silencieux). Le registre ne
/// contient que du **public** → la re-preuve reste **hors ligne**, sans coopération de personne.
pub fn verify_run_offline_with_nodes(
    swarm_records: &[AgentCallRecord],
    n17_records: &[LlmProvenanceRecord],
    n17_leaves: &[[u8; 32]],
    n17_root: Option<[u8; 32]>,
    key: &[u8],
    node_registry: &NodeRegistry,
) -> OfflineVerdict {
    let swarm = verify_swarm(swarm_records, key);
    let n17_chain = verify_n17_chain(n17_records, n17_leaves, n17_root, key);
    // Le seam déjà prévu : un résolveur pré-chargé depuis les records N17 persistés.
    let resolver = MapResolver::from_records(n17_records.iter());
    let bridge = verify_bridge(swarm_records, &resolver);
    let fully_proven = swarm.ok() && n17_chain.intact && bridge.all_mode_c_consistent();
    // RC-1/RC-2 (I-RC8/I-RC9) : le barreau tenu est le plus faible des barreaux VÉRIFIÉS — une
    // revendication R3 non prouvée redescend à R4 — et tout écart est signalé, jamais sur parole.
    // Un R3 n'est honoré que sur un run dont la chaîne N17 est INTÈGRE (sinon le record n'est pas
    // prouvé appartenir au run) : `held_strength` n'est jamais découplé de l'intégrité de chaîne.
    let (held_strength, cosign_anomalies) =
        assess_held_strength(n17_records, node_registry, n17_chain.intact);
    OfflineVerdict { swarm, n17_chain, bridge, fully_proven, held_strength, cosign_anomalies }
}

/// **RC-2/R3** — évalue le barreau de computation **vérifié** de chaque record. Une revendication R3
/// n'est honorée (rang 2) que si TOUTES ces conditions tiennent ; sinon elle **redescend à R4** et
/// l'anomalie est collectée (I-RC9, jamais silencieux) :
///  1. la chaîne N17 du run est **intègre** (`chain_intact`) — sinon le record n'est pas prouvé du run ;
///  2. son nonce-défi n'est pas **réutilisé** par un autre record R3 du run (rejeu intra-run) ;
///  3. le record a une **sortie** (pas de substitution `ZERO` pour un `output_hash` absent) ;
///  4. la co-signature **vérifie** (ed25519, tuple recalculé, clé pinnée non révoquée).
///
/// Pur, hors ligne. (La fraîcheur INTER-run du nonce — défi unique jamais rejoué d'un run passé —
/// reste hors de portée hors ligne ; c'est la garantie d'émission orchestrateur, cf. I-RC12.)
fn assess_held_strength(
    records: &[LlmProvenanceRecord],
    registry: &NodeRegistry,
    chain_intact: bool,
) -> (HeldStrength, Vec<CosignAnomaly>) {
    // Pré-passe : compter les nonces-défis des records R3 — un nonce réutilisé dans le run signale
    // un rejeu intra-run. Indépendant de l'ordre (déterministe) ; degrade TOUS les doublons.
    let mut nonce_counts: HashMap<&str, usize> = HashMap::new();
    for r in records {
        if let ComputeAttestation::R3RemoteCosigned { cosignature, .. } = &r.compute_attestation {
            *nonce_counts.entry(cosignature.nonce.as_str()).or_insert(0) += 1;
        }
    }

    let mut weakest_rank: Option<u8> = None;
    let mut weakest_label: Option<String> = None;
    let mut any_remote = false;
    let mut anomalies = Vec::new();
    for r in records {
        let a = &r.compute_attestation;
        any_remote |= a.is_remote();
        let (eff_rank, eff_label): (u8, &'static str) = match a {
            ComputeAttestation::R3RemoteCosigned { cosignature, .. } => {
                // Cascade « aucune élévation sans preuve » (I-RC9) : le premier obstacle gagne.
                let downgrade: Option<CosignDowngrade> = if !chain_intact {
                    Some(CosignDowngrade::ChainNotIntact)
                } else if nonce_counts.get(cosignature.nonce.as_str()).copied().unwrap_or(0) > 1 {
                    Some(CosignDowngrade::NonceReused)
                } else {
                    match r.output_hash {
                        None => Some(CosignDowngrade::MissingOutput),
                        Some(oh) => {
                            verify_cosignature(&r.final_prompt_hash, &oh, cosignature, registry).err()
                        }
                    }
                };
                match downgrade {
                    None => (2, "R3-DISTANT-COSIGNE"),
                    Some(reason) => {
                        anomalies.push(CosignAnomaly {
                            provenance_id: r.provenance_id,
                            node_id: cosignature.node_id.clone(),
                            reason,
                        });
                        (1, "R4-DISTANT-CANAL") // redescendu (I-RC9), jamais silencieux
                    }
                }
            }
            other => (other.strength_rank(), other.rung_label()),
        };
        let lower = match weakest_rank {
            Some(w) => eff_rank < w,
            None => true,
        };
        if lower {
            weakest_rank = Some(eff_rank);
            weakest_label = Some(eff_label.to_string());
        }
    }
    (HeldStrength { weakest_rank, weakest_label, any_remote }, anomalies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosign::Attestor; // les méthodes cosign()/public_key_hex() du MockAttestor
    use efe_nano_sdk::{
        AgentId, Blake3Hash, Completeness, ComputeAttestation, CostAccounting, EndpointIdentity,
        FinishReason, ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, RetryOutcome,
        SamplingParams, SwarmId, TenantId, TokenUsage,
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
            compute_attestation: ComputeAttestation::default(),
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
    fn injected_record_breaks_root_via_count_and_localizes() {
        // Direction SYMÉTRIQUE de deleted_record_breaks_root : relire PLUS de records que de
        // feuilles scellées (un N17 orphelin injecté que l'attaquant ne peut pas re-sceller).
        // Éprouve la moitié `>` de la cardinalité ET la localisation first_break en CountMismatch
        // (jamais asserties ailleurs : le doc-comment promet « ajout/suppression »).
        let recs = three();
        let sealed = seal_n17(&recs[..2], &key()); // ne scelle QUE 2 feuilles
        let v = verify_n17_chain(&recs, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact);
        assert_eq!(v.reason, Some(N17Break::CountMismatch));
        assert_eq!(v.records, 3, "le compte relu (3) > feuilles scellées (2)");
        let first_canonical = recs.iter().min_by_key(|r| r.provenance_id.as_uuid()).unwrap().provenance_id;
        assert_eq!(v.first_break, Some(first_canonical), "first_break pointe le 1er record en ordre canonique");
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
        // RC-1 : ce run est LOCAL (n17() ⇒ défaut Legacy ici) — held_strength le rapporte.
        assert!(!v.held_strength.any_remote);
    }

    // ── RC-1 : le gate du Mode C distant re-prouvable (CI-pur, distant SIMULÉ) ────────

    /// Construit un record N17 estampillé d'un barreau distant R4 (canal pinné), comme le
    /// ferait `build_local_backend` pour un endpoint distant épinglé.
    fn n17_remote(prov: u128, tenant: TenantId, swarm: Uuid, host: &str) -> LlmProvenanceRecord {
        let mut r = n17(prov, tenant, swarm, "gemma3:12b");
        r.compute_attestation = ComputeAttestation::R4RemoteChannel {
            endpoint: EndpointIdentity::new(host, "sha256/SIMULATED-REMOTE-PIN"),
            keyed: false,
        };
        r
    }

    #[test]
    fn remote_run_reproves_at_r4_but_held_strength_is_reported_not_folded() {
        use crate::record::{AgentCallDraft, AgentKind, AgentRole};
        // [SIMULATED REMOTE] — l'inférence a (conceptuellement) quitté la boîte : un record R4.
        // I-RC8 : la TRANSACTION re-prouve (fully_proven), MAIS le vérifieur apprend que le
        // barreau de computation tenu est R4 — jamais replié dans un `true` nu.
        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(swarm_uuid());
        let n17_recs = vec![n17_remote(1, tenant, swarm_uuid(), "gpu.internal")];
        let sealed = seal_n17(&n17_recs, &key());
        let draft = AgentCallDraft::new(
            swarm, tenant, AgentId::new("generator").unwrap(), AgentRole::Generator,
            AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_llm_provenance(n17_recs[0].provenance_id);
        let mut chain = crate::chain::SwarmChain::new(tenant, swarm, key());
        let swarm_recs = chain.append_all(std::iter::once(draft));

        let v = verify_run_offline(&swarm_recs, &n17_recs, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(v.fully_proven, "la TRANSACTION distante re-prouve (R0)");
        assert_eq!(v.held_strength.weakest_rank, Some(1), "barreau tenu = R4 (rang 1)");
        assert_eq!(v.held_strength.weakest_label.as_deref(), Some("R4-DISTANT-CANAL"));
        assert!(v.held_strength.any_remote, "l'inférence a quitté la boîte souveraine");
    }

    #[test]
    fn held_strength_is_the_weakest_link_across_mixed_rungs() {
        // Un run mixte (un appel local R2 + un appel distant R4) tient la confiance du MAILLON
        // LE PLUS FAIBLE : R4. (Le « held_strength » est un minimum, pas une moyenne.)
        let tenant = TenantId::system();
        let mut local = n17(1, tenant, swarm_uuid(), "gemma3:12b");
        local.compute_attestation = ComputeAttestation::R2LocalWitnessed;
        let remote = n17_remote(2, tenant, swarm_uuid(), "gpu.internal");
        let hs = HeldStrength::from_records(&[local, remote]);
        assert_eq!(hs.weakest_rank, Some(1), "le maillon faible (R4) gouverne");
        assert_eq!(hs.weakest_label.as_deref(), Some("R4-DISTANT-CANAL"));
        assert!(hs.any_remote);
    }

    #[test]
    fn downgrading_a_sealed_remote_rung_to_local_breaks_the_chain() {
        // LE test anti-downgrade (I-RC3 / §7). Un attaquant scelle un record DISTANT (R4) puis
        // tente de le faire passer pour LOCAL (R2) après coup pour sur-prétendre. Comme le
        // barreau est dans le corps canonique → dans le leaf, la réécriture casse la chaîne N17
        // et LOCALISE le record fautif. Aucun downgrade silencieux possible.
        let tenant = TenantId::system();
        let recs = vec![
            n17(1, tenant, swarm_uuid(), "gemma3:12b"),
            n17_remote(2, tenant, swarm_uuid(), "gpu.internal"),
            n17(3, tenant, swarm_uuid(), "gemma3:12b"),
        ];
        let sealed = seal_n17(&recs, &key());
        // l'attaque : R4-DISTANT-CANAL → R2-LOCAL-TEMOIN (mensonge de localité).
        let mut tampered = recs.clone();
        tampered[1].compute_attestation = ComputeAttestation::R2LocalWitnessed;
        let v = verify_n17_chain(&tampered, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact, "le downgrade de localité doit casser la chaîne");
        assert_eq!(v.reason, Some(N17Break::RecordContentMutated));
        assert_eq!(v.first_break, Some(recs[1].provenance_id), "le record downgradé est localisé");
    }

    // ── RC-2 / R3 : la co-signature T-WITNESS, bout-en-bout [SIMULATED ATTESTOR] ──────

    /// Construit un record N17 estampillé R3, co-signé par `att` sur les hashes RÉELS du record.
    fn n17_r3(prov: u128, tenant: TenantId, att: &crate::cosign::MockAttestor, nonce: &str) -> LlmProvenanceRecord {
        let mut r = n17(prov, tenant, swarm_uuid(), "gemma3:12b");
        let output_hash = r.output_hash.unwrap_or(Blake3Hash::ZERO);
        let cosig = att.cosign(&r.final_prompt_hash, &output_hash, nonce);
        r.compute_attestation = ComputeAttestation::R3RemoteCosigned {
            endpoint: EndpointIdentity::new("gpu.internal", "sha256/CHANNELPIN"),
            cosignature: cosig,
        };
        r
    }

    /// Un run Mode C minimal (1 agent lié à 1 record N17), scellé essaim + N17 — pour exercer
    /// `verify_run_offline*` de bout en bout.
    fn run_with(n17_rec: LlmProvenanceRecord) -> (Vec<AgentCallRecord>, Vec<LlmProvenanceRecord>, SealedN17) {
        use crate::record::{AgentCallDraft, AgentKind, AgentRole};
        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(swarm_uuid());
        let n17_recs = vec![n17_rec];
        let sealed = seal_n17(&n17_recs, &key());
        let draft = AgentCallDraft::new(
            swarm, tenant, AgentId::new("generator").unwrap(), AgentRole::Generator,
            AgentKind::ModeC, Blake3Hash::from_bytes([1u8; 32]), Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_llm_provenance(n17_recs[0].provenance_id);
        let mut chain = crate::chain::SwarmChain::new(tenant, swarm, key());
        let swarm_recs = chain.append_all(std::iter::once(draft));
        (swarm_recs, n17_recs, sealed)
    }

    #[test]
    fn r3_run_holds_at_r3_when_the_cosignature_verifies() {
        // Nominal : un nœud pinné co-signe → le vérifieur HONORE R3 (rang 2), zéro anomalie.
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let (sw, n17r, sealed) = run_with(n17_r3(1, TenantId::system(), &att, "challenge-0001"));
        let v = verify_run_offline_with_nodes(&sw, &n17r, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert!(v.fully_proven, "la transaction re-prouve (R0)");
        assert_eq!(v.held_strength.weakest_rank, Some(2), "co-signature vérifiée → R3 (rang 2)");
        assert_eq!(v.held_strength.weakest_label.as_deref(), Some("R3-DISTANT-COSIGNE"));
        assert!(v.held_strength.any_remote);
        assert!(v.cosign_anomalies.is_empty(), "aucune sur-prétention");
    }

    #[test]
    fn r3_claim_without_a_node_registry_degrades_to_r4() {
        // I-RC9 : sans registre (verify_run_offline nu), R3 est invérifiable → redescend à R4,
        // anomalie NodeNotPinned. La transaction re-prouve toujours.
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let (sw, n17r, sealed) = run_with(n17_r3(1, TenantId::system(), &att, "c"));
        let v = verify_run_offline(&sw, &n17r, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(v.fully_proven);
        assert_eq!(v.held_strength.weakest_rank, Some(1), "aucune clé pinnée → R3 non tenu → R4");
        assert_eq!(v.cosign_anomalies.len(), 1);
        assert_eq!(v.cosign_anomalies[0].reason, CosignDowngrade::NodeNotPinned);
        assert_eq!(v.cosign_anomalies[0].node_id, "gpu-1");
    }

    #[test]
    fn r3_forged_cosignature_degrades_to_r4_with_anomaly() {
        // Sur-prétention : le record porte une co-signature d'un IMPOSTEUR (même node_id, autre
        // clé) ; le vérifieur pinne la VRAIE clé → la signature ne vérifie pas → R4 + anomalie.
        let real = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let imposter = crate::cosign::MockAttestor::from_seed("gpu-1", [9u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", real.public_key_hex());
        let (sw, n17r, sealed) = run_with(n17_r3(1, TenantId::system(), &imposter, "c"));
        let v = verify_run_offline_with_nodes(&sw, &n17r, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert!(v.fully_proven, "la transaction re-prouve, mais le témoin est rejeté");
        assert_eq!(v.held_strength.weakest_rank, Some(1), "signature forgée → R4");
        assert_eq!(v.cosign_anomalies[0].reason, CosignDowngrade::SignatureInvalid);
    }

    #[test]
    fn r3_revoked_node_degrades_to_r4() {
        // I-RC13 : un nœud dont la clé est révoquée ne hisse plus ses records — redescente R4.
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let mut registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        registry.revoke("gpu-1", "clé compromise");
        let (sw, n17r, sealed) = run_with(n17_r3(1, TenantId::system(), &att, "c"));
        let v = verify_run_offline_with_nodes(&sw, &n17r, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert_eq!(v.held_strength.weakest_rank, Some(1), "nœud révoqué → R4");
        assert_eq!(v.cosign_anomalies[0].reason, CosignDowngrade::NodeRevoked);
    }

    #[test]
    fn sealed_r3_cosignature_cannot_be_stripped() {
        // Défense en profondeur : la co-signature est DANS le leaf (RC-1). La retirer (R3 → R4 nu)
        // pour effacer le témoin casse la chaîne N17 et localise le record.
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let recs = vec![n17_r3(1, TenantId::system(), &att, "c")];
        let sealed = seal_n17(&recs, &key());
        let mut tampered = recs.clone();
        tampered[0].compute_attestation = ComputeAttestation::R4RemoteChannel {
            endpoint: EndpointIdentity::new("gpu.internal", "sha256/CHANNELPIN"),
            keyed: false,
        };
        let v = verify_n17_chain(&tampered, &sealed.leaf_hashes, sealed.n17_root, &key());
        assert!(!v.intact, "retirer la co-signature scellée casse la chaîne");
        assert_eq!(v.reason, Some(N17Break::RecordContentMutated));
        assert_eq!(v.first_break, Some(recs[0].provenance_id));
    }

    // ── RC-2 (revue adversariale) : les durcissements de la passe de vérification ─────

    #[test]
    fn r3_with_missing_output_degrades_to_r4_no_zero_substitution() {
        // Un record R3 SANS sortie ne doit pas voir sa co-signature liée à un contenu ZERO fabriqué
        // (findings no-elevation / tuple-binding) : il redescend à R4 + anomalie MissingOutput.
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let mut rec = n17_r3(1, TenantId::system(), &att, "challenge-1");
        rec.output_hash = None; // pas de sortie
        let (sw, n17r, sealed) = run_with(rec);
        let v = verify_run_offline_with_nodes(&sw, &n17r, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert_eq!(v.held_strength.weakest_rank, Some(1), "sortie absente → pas de R3");
        assert_eq!(v.cosign_anomalies[0].reason, CosignDowngrade::MissingOutput);
    }

    #[test]
    fn r3_replayed_nonce_within_a_run_degrades_both_to_r4() {
        // I-RC12 (volet hors-ligne) : deux records R3 partageant le MÊME nonce-défi = rejeu
        // intra-run. AUCUN des deux n'est honoré (on ne sait pas lequel est légitime).
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        // deux records distincts, MÊME nonce "REUSED" — chacun co-signé valablement.
        let recs = vec![
            n17_r3(1, TenantId::system(), &att, "REUSED"),
            n17_r3(2, TenantId::system(), &att, "REUSED"),
        ];
        let sealed = seal_n17(&recs, &key());
        // (les deux records sont liés à la chaîne d'essaim pour fully_proven ; ici on cible held_strength)
        let v = verify_run_offline_with_nodes(&[], &recs, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert_eq!(v.held_strength.weakest_rank, Some(1), "nonce rejoué intra-run → R4 pour les deux");
        assert_eq!(v.cosign_anomalies.len(), 2, "les DEUX doublons sont signalés");
        assert!(v.cosign_anomalies.iter().all(|a| a.reason == CosignDowngrade::NonceReused));
    }

    #[test]
    fn distinct_nonces_in_a_run_are_both_honored() {
        // Contrôle : deux R3 à nonces DISTINCTS sont tous deux honorés (pas de faux positif).
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let recs = vec![
            n17_r3(1, TenantId::system(), &att, "challenge-1"),
            n17_r3(2, TenantId::system(), &att, "challenge-2"),
        ];
        let sealed = seal_n17(&recs, &key());
        let v = verify_run_offline_with_nodes(&[], &recs, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert_eq!(v.held_strength.weakest_rank, Some(2), "nonces distincts → R3 tenu");
        assert!(v.cosign_anomalies.is_empty());
    }

    #[test]
    fn r3_is_not_honored_when_the_n17_chain_is_broken() {
        // Couplage chaîne↔témoin : même avec une co-signature VALIDE, un R3 sur un run dont la
        // chaîne N17 est rompue n'est PAS honoré (le record n'est pas prouvé appartenir au run).
        let att = crate::cosign::MockAttestor::from_seed("gpu-1", [7u8; 32]);
        let registry = NodeRegistry::empty().with_node("gpu-1", att.public_key_hex());
        let recs = vec![n17_r3(1, TenantId::system(), &att, "challenge-1")];
        let sealed = seal_n17(&recs, &key());
        // on mute un champ HORS tuple (op_id) : la co-signature vérifierait encore, MAIS la chaîne casse.
        let mut tampered = recs.clone();
        tampered[0].op_id = "swarm.tampered".into();
        let v = verify_run_offline_with_nodes(&[], &tampered, &sealed.leaf_hashes, sealed.n17_root, &key(), &registry);
        assert!(!v.n17_chain.intact, "la chaîne est rompue");
        assert_eq!(v.held_strength.weakest_rank, Some(1), "chaîne rompue → R3 non honoré");
        assert_eq!(v.cosign_anomalies[0].reason, CosignDowngrade::ChainNotIntact);
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
