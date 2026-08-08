// SPDX-License-Identifier: UNLICENSED
//! **La dérivation** — un run scellé de la pile A devient les nœuds typés de la pile B.
//!
//! # Ce que cette fonction est, et n'est pas
//!
//! Elle est **pure** : aucune horloge, aucun aléa, aucun identifiant frais, aucune écriture. Le
//! même run produit les mêmes nœuds, à l'octet — donc le même manifest-root. C'est la condition
//! sans laquelle un bundle ne serait pas re-dérivable, et donc ne prouverait rien.
//!
//! Elle n'est **pas** une seconde source de vérité. Le DAG produit est une **vue** du reçu de la
//! pile A : il n'ajoute aucune information, il change le *vérifieur*. Cette phrase est le
//! garde-fou du fil entier — dès qu'un bundle prétend savoir quelque chose que le reçu ignore, le
//! pont est devenu un second producteur, et les deux peuvent diverger sans que personne ne le voie.
//!
//! # L'ordre d'émission, et pourquoi il n'est pas négociable
//!
//! La pile B impose la causalité : les identifiants cités par un nœud doivent **exister avant**
//! lui (`Node::refs`). Deux contraintes en découlent :
//! - une évidence doit précéder l'acte qui la cite ;
//! - une arête doit suivre ses **deux** extrémités.
//!
//! On émet donc, dans l'ordre canonique de la pile A : pour chaque enregistrement, son évidence
//! (le cas échéant) puis son acte ; **puis seulement** toutes les arêtes. Émettre les arêtes en
//! dernier rend l'ordre correct *quelle que soit* la topologie du run — y compris si un parent
//! apparaissait après son enfant dans l'ordre canonique. On ne suppose pas la topologie, on la
//! rend inoffensive.
//!
//! # Ce que la dérivation vérifie, et ce qu'elle laisse à la pile A
//!
//! Elle vérifie la **complétude** : la racine scellée doit se reproduire, sans quoi elle refuse.
//! Elle ne re-juge **pas** le pont N17 (Mode A ↔ Mode C) : c'est le verdict de la pile A, exposé
//! par sa route de vérification. Conséquence dite : un appel Mode C dont la forensique est
//! absente produit un acte **sans évidence** — le pont n'invente pas une évidence qu'il n'a pas,
//! et l'écart acte/évidence reste lisible dans le bundle.

use std::collections::{HashMap, HashSet};

use efe_nano_sdk::LlmProvenanceRecord;
use efe_swarm_provenance::{deterministic_order, verify_swarm};
use provenance_dag::Node;
use uuid::Uuid;

use crate::error::DeriveError;
use crate::mapping::{
    act_node_id, edge_node_id, evidence_node_id, millis_since_epoch, model_of, tagged_hash,
    HASH_ABSENT, MAPPING_VERSION,
};
use crate::view::RunView;

/// Le schéma d'URN qui relie une évidence du bundle à l'enregistrement forensique de la pile A.
///
/// Ce n'est pas décoratif : c'est le fil qu'un auditeur remonte pour passer du bundle au reçu.
const LLM_PROVENANCE_URN: &str = "llm-provenance";

/// **Dérive** les nœuds de la pile B depuis un run scellé de la pile A.
///
/// # Refus
///
/// Voir [`DeriveError`]. Aucun DAG partiel n'est jamais rendu : un bundle amputé serait
/// indistinguable d'un bundle tronqué par un attaquant.
///
/// # Complétude
///
/// La règle est celle de P.0, et pour la même raison : **c'est la racine scellée qui commande**.
/// Dès qu'un run en porte une, la re-preuve doit la reproduire. Un run *vide* portant une racine
/// est donc refusé (`RootMismatch` avec `recomputed: None`) — c'est le cas vicieux, puisque la
/// pile A déclare un run vide *trivialement intègre*.
pub fn nodes_from_run(view: &RunView<'_>, key: &[u8]) -> Result<Vec<Node>, DeriveError> {
    // ── (0) Complétude ─────────────────────────────────────────────────────────────────────
    // Un run vide SANS racine n'a rien à trahir : le DAG vide est sa vue fidèle.
    if view.is_empty() {
        return match view.sealed_root {
            None => Ok(Vec::new()),
            Some(sealed) => {
                Err(DeriveError::RootMismatch { sealed: sealed.to_string(), recomputed: None })
            }
        };
    }
    let sealed = view.sealed_root.ok_or(DeriveError::MissingRoot)?;

    // On re-prouve nous-mêmes plutôt que de croire l'appelant : le pont ne doit pas pouvoir être
    // convaincu par un paramètre qu'un run compromis est sain.
    let report = verify_swarm(view.records, key);
    let recomputed = report.root_hex();
    if recomputed.as_deref() != Some(sealed) {
        return Err(DeriveError::RootMismatch { sealed: sealed.to_string(), recomputed });
    }

    // ── (1) Ordre canonique ────────────────────────────────────────────────────────────────
    // Réutilisé, jamais recopié : une seconde implémentation dériverait, et deux ordres
    // canoniques divergents produiraient deux racines pour un même run.
    let ordered = deterministic_order(view.records);

    // ── (2) Index : record_id -> identifiant de nœud d'acte ────────────────────────────────
    // Construit AVANT toute émission, pour que les arêtes puissent résoudre leurs extrémités
    // sans dépendre de l'ordre, et pour détecter les doublons au plus tôt.
    let mut act_id_of: HashMap<Uuid, String> = HashMap::with_capacity(ordered.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(ordered.len() * 2);
    for r in &ordered {
        let id = act_node_id(view.run_id, r);
        if !seen.insert(id.clone()) {
            return Err(DeriveError::DuplicateNodeId(id));
        }
        act_id_of.insert(r.record_id, id);
    }

    // ── (3) Index forensique : provenance_id -> enregistrement LLM ─────────────────────────
    let llm_by_id: HashMap<Uuid, &LlmProvenanceRecord> =
        view.llm_records.iter().map(|l| (l.provenance_id.as_uuid(), l)).collect();

    // ── (4) Émission : (évidence?, acte) dans l'ordre, puis les arêtes ─────────────────────
    let mut nodes: Vec<Node> = Vec::with_capacity(ordered.len() * 2);
    let mut edges: Vec<Node> = Vec::new();

    for r in &ordered {
        let act_id = act_id_of
            .get(&r.record_id)
            .expect("indexé en (2) : tout enregistrement ordonné a son identifiant")
            .clone();
        let ts = millis_since_epoch(r.wall_at)
            .ok_or(DeriveError::TimestampBeforeEpoch { record: r.record_id })?;

        // (4a) L'évidence, si — et seulement si — la forensique est réellement là.
        let mut evidence_refs: Vec<String> = Vec::new();
        let mut model_identity: Option<(&str, &str)> = None;
        if let Some(pid) = r.llm_provenance_id {
            if let Some(llm) = llm_by_id.get(&pid.as_uuid()) {
                let ev_id = evidence_node_id(&act_id);
                if !seen.insert(ev_id.clone()) {
                    return Err(DeriveError::DuplicateNodeId(ev_id));
                }
                model_identity =
                    Some((llm.model_identity.provider.as_str(), llm.model_identity.model_id.as_str()));
                nodes.push(Node::Evidence {
                    evidence_id: ev_id.clone(),
                    tool: llm.model_identity.provider.clone(),
                    query_hash: tagged_hash(&llm.final_prompt_hash.to_hex()),
                    retrieved_ref: format!("{LLM_PROVENANCE_URN}://{}", pid.as_uuid()),
                    content_hash: llm
                        .output_hash
                        .as_ref()
                        .map_or_else(|| HASH_ABSENT.to_string(), |h| tagged_hash(&h.to_hex())),
                    // Pointeur-retour scellé (annotation, pas prérequis causal — cf. pile B).
                    consumed_by_node: act_id.clone(),
                    timestamp: ts,
                });
                evidence_refs.push(ev_id);
            }
            // Sinon : Mode C sans forensique résolue. On n'invente pas d'évidence ; l'acte porte
            // quand même sa nature `mode_c`. L'écart est le verdict de la pile A, pas le nôtre.
        }

        // (4b) L'acte.
        nodes.push(Node::AgentAct {
            node_id: act_id.clone(),
            agent_role: r.agent_role.as_str().to_string(),
            inputs_hash: tagged_hash(&r.input_hash.to_hex()),
            model: model_of(r, model_identity),
            version: MAPPING_VERSION.to_string(),
            // Une EMPREINTE, pas la sortie — le préfixe l'interdit de toute confusion.
            output: tagged_hash(&r.output_hash.to_hex()),
            evidence_refs,
            timestamp: ts,
        });

        // (4c) Les arêtes causales, mises de côté jusqu'à ce que tous les actes existent.
        for parent in &r.causal_parents {
            let from = act_id_of
                .get(parent)
                .ok_or_else(|| DeriveError::PendingRef { node: act_id.clone(), missing: *parent })?;
            let parent_rec = ordered
                .iter()
                .find(|p| p.record_id == *parent)
                .expect("résolu juste au-dessus dans le même ensemble ordonné");
            let edge_id = edge_node_id(from, &act_id);
            if !seen.insert(edge_id.clone()) {
                return Err(DeriveError::DuplicateNodeId(edge_id));
            }
            edges.push(Node::Edge {
                edge_id,
                from_node: from.clone(),
                to_node: act_id.clone(),
                // Ce qui a transité, c'est la SORTIE du parent — son empreinte.
                artifact_hash: tagged_hash(&parent_rec.output_hash.to_hex()),
                artifact_type: parent_rec.agent_role.as_str().to_string(),
                timestamp: ts,
            });
        }
    }

    nodes.extend(edges);
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{act_node_id, evidence_node_id};
    use efe_nano_sdk::{AgentId, Blake3Hash, ProvenanceId, SwarmId, TenantId};
    use efe_swarm_provenance::{AgentCallDraft, AgentCallRecord, AgentKind, AgentRole, SwarmChain};
    use provenance_dag::{reprove_dag, SealedDag, TenantKey};
    use std::time::{Duration, UNIX_EPOCH};

    fn key() -> Vec<u8> {
        vec![7u8; 32]
    }

    /// Un run RÉEL : vraie chaîne HMAC, vraie racine. Jamais de doublure — le pont doit être
    /// éprouvé contre ce qu'il rencontrera.
    ///
    /// `n` actes Mode A en chaîne causale (chacun causé par le précédent), horodatés de façon
    /// déterministe.
    fn chained_run(n: usize) -> (Uuid, Vec<AgentCallRecord>, String) {
        let tenant = TenantId::system();
        let swarm = SwarmId::new();
        let mut chain = SwarmChain::new(tenant, swarm, key());
        let mut drafts = Vec::new();
        let mut prev: Option<Uuid> = None;
        for i in 0..n {
            let rid = Uuid::from_u128(0xA000_0000_0000_0000_0000_0000_0000_0000u128 + i as u128);
            let mut d = AgentCallDraft::new(
                swarm,
                tenant,
                AgentId::new(format!("agent{i}")).unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                Blake3Hash::from_bytes([i as u8; 32]),
                Blake3Hash::from_bytes([(i + 100) as u8; 32]),
            )
            .with_record_id(rid)
            .with_wall_at(UNIX_EPOCH + Duration::from_millis(1_000 + i as u64))
            .wrote(format!("cell{i}"), 1);
            if let Some(p) = prev {
                d = d.caused_by(p);
            }
            prev = Some(rid);
            drafts.push(d);
        }
        let records = chain.append_all_canonical(drafts);
        (Uuid::from_u128(0xBEEF), records, chain.root_hex().expect("chaîne non vide"))
    }

    // ── Le contrat nominal ────────────────────────────────────────────────────────────────

    #[test]
    fn derives_acts_and_edges_in_a_causally_valid_order() {
        let (run, recs, root) = chained_run(3);
        let view = RunView::new(run, "t", &recs, &[], Some(&root));
        let nodes = nodes_from_run(&view, &key()).expect("run sain");

        // 3 actes + 2 arêtes (chaîne linéaire).
        assert_eq!(nodes.len(), 5, "3 actes + 2 arêtes");
        let acts = nodes.iter().filter(|n| matches!(n, Node::AgentAct { .. })).count();
        let edges = nodes.iter().filter(|n| matches!(n, Node::Edge { .. })).count();
        assert_eq!((acts, edges), (3, 2));

        // Les arêtes sont EN QUEUE : leurs deux extrémités existent déjà.
        let first_edge = nodes.iter().position(|n| matches!(n, Node::Edge { .. })).unwrap();
        let last_act = nodes.iter().rposition(|n| matches!(n, Node::AgentAct { .. })).unwrap();
        assert!(last_act < first_edge, "toutes les arêtes suivent tous les actes");
    }

    #[test]
    fn the_derived_dag_is_accepted_and_reproves() {
        // Le test qui compte : la pile B ACCEPTE la dérivation (causalité, unicité) et la
        // re-prouve. Sans lui, tout le reste n'est que du typage.
        let (run, recs, root) = chained_run(4);
        let view = RunView::new(run, "t", &recs, &[], Some(&root));
        let nodes = nodes_from_run(&view, &key()).unwrap();

        let bkey = TenantKey::derive(b"racine-de-test", "tenant-t");
        let mut dag = SealedDag::new();
        for n in nodes {
            dag.append(&bkey, n).expect("la pile B accepte chaque nœud dérivé");
        }
        let verdict = reprove_dag(dag.records(), &bkey);
        assert!(verdict.ok, "le bundle dérivé se re-prouve : {:?}", verdict.reason);
        assert_eq!(verdict.n_records, 7, "4 actes + 3 arêtes (chaîne linéaire)");
    }

    #[test]
    fn derivation_is_deterministic_and_input_order_independent() {
        let (run, recs, root) = chained_run(4);
        let a = nodes_from_run(&RunView::new(run, "t", &recs, &[], Some(&root)), &key()).unwrap();

        // Même entrée → même sortie.
        let b = nodes_from_run(&RunView::new(run, "t", &recs, &[], Some(&root)), &key()).unwrap();
        assert_eq!(a, b, "dérivation déterministe");

        // Entrée MÉLANGÉE → même sortie : c'est `deterministic_order` qui parle.
        let mut shuffled = recs.clone();
        shuffled.reverse();
        let c =
            nodes_from_run(&RunView::new(run, "t", &shuffled, &[], Some(&root)), &key()).unwrap();
        assert_eq!(a, c, "l'ordre de transmission n'influence pas la dérivation");
    }

    #[test]
    fn a_fingerprint_can_never_pass_for_an_output() {
        let (run, recs, root) = chained_run(1);
        let nodes =
            nodes_from_run(&RunView::new(run, "t", &recs, &[], Some(&root)), &key()).unwrap();
        match &nodes[0] {
            Node::AgentAct { output, inputs_hash, version, model, .. } => {
                assert!(output.starts_with("blake3:"), "la sortie est une EMPREINTE, et le dit");
                assert!(inputs_hash.starts_with("blake3:"));
                assert_eq!(version, MAPPING_VERSION, "le nœud date la RÈGLE qui l'a produit");
                assert_eq!(model, "mode_a", "un agent Rust n'a pas de modèle — on ne l'invente pas");
            }
            other => panic!("attendu un acte, obtenu {other:?}"),
        }
    }

    // ── Les refus ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn refuses_a_truncated_run_rather_than_whitewash_it() {
        // LE refus qui porte le fil : donner un bundle re-prouvable à un run tronqué
        // reviendrait à offrir un témoin crédible à un reçu menteur.
        let (run, recs, root) = chained_run(4);
        let truncated = &recs[..3];
        let err = nodes_from_run(&RunView::new(run, "t", truncated, &[], Some(&root)), &key())
            .expect_err("un run tronqué ne doit PAS produire de bundle");
        match err {
            DeriveError::RootMismatch { sealed, recomputed } => {
                assert_eq!(sealed, root);
                assert!(recomputed.is_some(), "le préfixe reste intègre — c'est bien la RACINE qui trahit");
                assert_ne!(recomputed.unwrap(), root);
            }
            other => panic!("attendu RootMismatch, obtenu {other:?}"),
        }
    }

    #[test]
    fn refuses_an_emptied_run_that_still_claims_a_root() {
        // Le cas vicieux : la pile A déclare un run VIDE trivialement intègre et ne reconstruit
        // aucune racine. Une implémentation naïve (comparer deux Some) le laisserait passer.
        let (run, _recs, root) = chained_run(3);
        let err = nodes_from_run(&RunView::new(run, "t", &[], &[], Some(&root)), &key())
            .expect_err("un run vidé conserve sa racine scellée : il doit être refusé");
        assert!(matches!(err, DeriveError::RootMismatch { recomputed: None, .. }));
    }

    #[test]
    fn an_empty_run_without_a_root_derives_to_an_empty_dag() {
        // Symétrique du précédent : sans racine, il n'y a rien à trahir.
        let nodes = nodes_from_run(&RunView::new(Uuid::nil(), "t", &[], &[], None), &key()).unwrap();
        assert!(nodes.is_empty(), "rien à sceller, rien à inventer");
    }

    #[test]
    fn refuses_a_run_whose_completeness_cannot_be_established() {
        let (run, recs, _root) = chained_run(2);
        let err = nodes_from_run(&RunView::new(run, "t", &recs, &[], None), &key())
            .expect_err("sans racine scellée, la complétude est indécidable");
        assert_eq!(err, DeriveError::MissingRoot);
    }

    #[test]
    fn refuses_a_tampered_run() {
        let (run, mut recs, root) = chained_run(3);
        recs[1].output_hash = Blake3Hash::from_bytes([0xff; 32]);
        let err = nodes_from_run(&RunView::new(run, "t", &recs, &[], Some(&root)), &key())
            .expect_err("un run altéré ne se re-prouve pas");
        // La chaîne casse ⟹ aucune racine reconstruite.
        assert!(matches!(err, DeriveError::RootMismatch { recomputed: None, .. }));
    }

    #[test]
    fn refuses_a_pre_epoch_timestamp_rather_than_rounding_it() {
        let tenant = TenantId::system();
        let swarm = SwarmId::new();
        let mut chain = SwarmChain::new(tenant, swarm, key());
        let d = AgentCallDraft::new(
            swarm,
            tenant,
            AgentId::new("pre-epoch").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeA,
            Blake3Hash::from_bytes([1; 32]),
            Blake3Hash::from_bytes([2; 32]),
        )
        .with_wall_at(UNIX_EPOCH - Duration::from_secs(1));
        let recs = chain.append_all_canonical(vec![d]);
        let root = chain.root_hex().unwrap();
        let err = nodes_from_run(&RunView::new(Uuid::nil(), "t", &recs, &[], Some(&root)), &key())
            .expect_err("une horloge qui ment est un fait à dire");
        assert!(matches!(err, DeriveError::TimestampBeforeEpoch { .. }));
    }

    #[test]
    fn refuses_a_dangling_causal_parent() {
        // Un enregistrement cite un parent absent : la pile B rejetterait l'arête ; le pont le dit
        // avant, avec les deux identifiants.
        let (run, recs, root) = chained_run(3);
        // On retire le PREMIER (le parent du deuxième) — mais on garde la racine d'origine, donc
        // il faut re-sceller pour isoler le cas « référence pendante » de « racine fausse ».
        let orphan = vec![recs[1].clone(), recs[2].clone()];
        let rep = verify_swarm(&orphan, &key());
        let err = nodes_from_run(
            &RunView::new(run, "t", &orphan, &[], rep.root_hex().as_deref().or(Some(&root))),
            &key(),
        )
        .expect_err("un parent absent doit être refusé");
        // Selon que la sous-chaîne se recompose ou non, on obtient l'un ou l'autre refus — les
        // deux sont des refus, et c'est le point : on ne dérive JAMAIS un run incohérent.
        assert!(
            matches!(err, DeriveError::PendingRef { .. } | DeriveError::RootMismatch { .. }),
            "attendu un refus typé, obtenu {err:?}"
        );
    }

    // ── LE GOLDEN DU MAPPING ──────────────────────────────────────────────────────────────

    /// Une empreinte attendue, telle que le mapping doit l'écrire : préfixée, 64 chiffres.
    fn h(byte: u8) -> String {
        format!("blake3:{}", format!("{byte:02x}").repeat(32))
    }

    /// La forensique d'un appel frontière, **entièrement figée**.
    fn frozen_llm(prov: ProvenanceId) -> efe_nano_sdk::LlmProvenanceRecord {
        use efe_nano_sdk::{
            Completeness, CostAccounting, FinishReason, LlmProvenanceRecord, ModelIdentity,
            NanoAgentId, NanoAgentVersion, PiiStrategy, RetryOutcome, SamplingParams, TokenUsage,
        };
        LlmProvenanceRecord {
            provenance_id: prov,
            trace_id: Uuid::from_u128(0xBEEF),
            swarm_request_id: Uuid::from_u128(0x5EED),
            op_id: "swarm.writer".into(),
            tenant_id: TenantId::system(),
            agent_id: NanoAgentId::new("writer").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: ModelIdentity::hosted("ollama", "gemma3:12b", "local"),
            model_hash_verified: false,
            prompt_template_id: "t".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::ZERO,
            system_prompt_hash: Blake3Hash::ZERO,
            // Distinct de l'input du record : le golden doit pouvoir dire LEQUEL a été pris.
            user_prompt_hash: Blake3Hash::from_bytes([7u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([7u8; 32]),
            sampling: SamplingParams {
                temperature: 0.2,
                top_p: None,
                max_tokens: 16,
                seed: Some(7),
                stop_sequences: vec![],
            },
            token_usage: TokenUsage {
                client_estimated_input: 5,
                client_estimated_output: 2,
                provider_reported_input: None,
                provider_reported_output: None,
                is_estimated: true,
            },
            cost_accounting: CostAccounting {
                cost_cents: 0.0,
                prices_table_version: 0,
                provider_quoted_cost_cents: None,
                overflow_capped: false,
            },
            provider_request_id: None,
            provider_response_time_ms: 5,
            provider_finish_reason: FinishReason::Stop,
            provider_region: None,
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
            provider_metadata: std::collections::HashMap::new(),
            output_hash: Some(Blake3Hash::from_bytes([8u8; 32])),
            completeness: Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            pii_strategy_applied: PiiStrategy::None,
            redaction_token_set: vec![],
            replay_of: None,
            cache_hit: None,
            recorded_at: UNIX_EPOCH,
            streaming_mode: false,
            retry_count: 0,
            retry_outcome: RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
            compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
        }
    }

    #[test]
    fn golden_mapping_is_frozen_to_the_byte() {
        // **Le test que le §9 désigne comme le plus important du Pont** : un run entièrement figé
        // produit EXACTEMENT cette liste de nœuds. Il n'a l'air de rien et il porte tout : le
        // mapping est ce qui décide de l'identité et du contenu des nœuds SCELLÉS. Une dérive
        // silencieuse y changerait toutes les racines des bundles futurs, sans qu'aucun autre test
        // ne s'en aperçoive — ils vérifient tous des propriétés (déterminisme, causalité,
        // re-preuve) que le mapping conserverait en dérivant.
        //
        // Toute modification de ce littéral doit être un choix DÉLIBÉRÉ, accompagné d'une montée
        // de MAPPING_VERSION.
        let tenant = TenantId::system();
        let swarm = SwarmId::from_uuid(Uuid::from_u128(0x5EED));
        let prov = ProvenanceId::from_uuid(Uuid::from_u128(0xC0FFEE));
        let run = Uuid::from_u128(0x42);
        let r0 = Uuid::from_u128(0x10);
        let r1 = Uuid::from_u128(0x11);

        let mut chain = SwarmChain::new(tenant, swarm, key());
        let d0 = AgentCallDraft::new(
            swarm,
            tenant,
            AgentId::new("planner").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeA,
            Blake3Hash::from_bytes([1u8; 32]),
            Blake3Hash::from_bytes([2u8; 32]),
        )
        .with_record_id(r0)
        .with_wall_at(UNIX_EPOCH + Duration::from_millis(1_000))
        .wrote("plan", 1);
        let d1 = AgentCallDraft::new(
            swarm,
            tenant,
            AgentId::new("writer").unwrap(),
            AgentRole::Solver,
            AgentKind::ModeC,
            Blake3Hash::from_bytes([3u8; 32]),
            Blake3Hash::from_bytes([4u8; 32]),
        )
        .with_record_id(r1)
        .with_wall_at(UNIX_EPOCH + Duration::from_millis(2_000))
        .caused_by(r0)
        .with_llm_provenance(prov);
        let records = chain.append_all_canonical(vec![d0, d1]);
        let root = chain.root_hex().unwrap();
        let llm = vec![frozen_llm(prov)];

        let nodes =
            nodes_from_run(&RunView::new(run, "acme", &records, &llm, Some(&root)), &key()).unwrap();

        let act0 = "00000000-0000-0000-0000-000000000042::000000-planner";
        let act1 = "00000000-0000-0000-0000-000000000042::000001-writer";
        let ev1 = "00000000-0000-0000-0000-000000000042::000001-writer#llm";

        let expected = vec![
            // (1) L'acte Mode A : aucun modèle inventé, l'empreinte de sortie préfixée.
            Node::AgentAct {
                node_id: act0.into(),
                agent_role: "generator".into(),
                inputs_hash: h(1),
                model: "mode_a".into(),
                version: MAPPING_VERSION.into(),
                output: h(2),
                evidence_refs: vec![],
                timestamp: 1_000,
            },
            // (2) L'évidence PRÉCÈDE l'acte qui la cite — la causalité de la pile B l'exige.
            Node::Evidence {
                evidence_id: ev1.into(),
                tool: "ollama".into(),
                query_hash: h(7), // le prompt FINAL, pas l'input du record
                retrieved_ref: format!("llm-provenance://{}", prov.as_uuid()),
                content_hash: h(8),
                consumed_by_node: act1.into(),
                timestamp: 2_000,
            },
            // (3) L'acte Mode C : le modèle porte fournisseur ET identifiant.
            Node::AgentAct {
                node_id: act1.into(),
                agent_role: "solver".into(),
                inputs_hash: h(3),
                model: "mode_c:ollama/gemma3:12b".into(),
                version: MAPPING_VERSION.into(),
                output: h(4),
                evidence_refs: vec![ev1.into()],
                timestamp: 2_000,
            },
            // (4) Les arêtes EN QUEUE : l'artefact transporté est la sortie du PARENT.
            Node::Edge {
                edge_id: format!("{act0}->{act1}"),
                from_node: act0.into(),
                to_node: act1.into(),
                artifact_hash: h(2),
                artifact_type: "generator".into(),
                timestamp: 2_000,
            },
        ];

        assert_eq!(nodes, expected, "le mapping a DÉRIVÉ — est-ce délibéré ? sinon, régression");
    }

    #[test]
    fn node_ids_are_injective_which_is_why_duplicates_are_unreachable() {
        // `DeriveError::DuplicateNodeId` ne peut PAS être atteint par un run qui passe le contrôle
        // de racine, et il faut le dire plutôt que d'écrire un test contorsionné : l'identité d'un
        // acte est `{run}::{seq:06}-{agent}`, or la chaîne de la pile A REFUSE un `seq` non
        // strictement croissant (`BreakReason::NonMonotonicSeq`) — deux records de même `seq`
        // cassent la chaîne, donc la racine, donc la dérivation refuse AVANT d'y arriver.
        //
        // La variante reste dans le code comme défense en profondeur : si le schéma d'identité
        // changeait un jour, elle transformerait une collision silencieuse en refus. Ce test
        // épingle la PROPRIÉTÉ qui la rend inatteignable, et sert d'alarme si elle disparaît.
        let (run, recs, _root) = chained_run(6);
        let ordered = deterministic_order(&recs);
        let ids: Vec<String> = ordered.iter().map(|r| act_node_id(run, r)).collect();
        let uniq: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(ids.len(), uniq.len(), "les identifiants d'actes sont injectifs");

        // Et les trois familles d'identifiants ne peuvent pas se croiser.
        for id in &ids {
            assert!(!uniq.contains(&evidence_node_id(id)), "aucune évidence ne collisionne un acte");
        }
        // Le `seq` étant strictement croissant, deux actes du même agent restent distincts.
        assert_ne!(act_node_id(run, &ordered[0]), act_node_id(run, &ordered[1]));
    }

    // ── Le chemin Mode C ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_mode_c_act_without_forensics_gets_no_invented_evidence() {
        let tenant = TenantId::system();
        let swarm = SwarmId::new();
        let mut chain = SwarmChain::new(tenant, swarm, key());
        let d = AgentCallDraft::new(
            swarm,
            tenant,
            AgentId::new("gen").unwrap(),
            AgentRole::Generator,
            AgentKind::ModeC,
            Blake3Hash::from_bytes([1; 32]),
            Blake3Hash::from_bytes([2; 32]),
        )
        .with_wall_at(UNIX_EPOCH + Duration::from_millis(5))
        .with_llm_provenance(ProvenanceId::new()); // …mais aucun LlmProvenanceRecord fourni.
        let recs = chain.append_all_canonical(vec![d]);
        let root = chain.root_hex().unwrap();

        let nodes =
            nodes_from_run(&RunView::new(Uuid::nil(), "t", &recs, &[], Some(&root)), &key()).unwrap();
        assert_eq!(nodes.len(), 1, "un acte, et AUCUNE évidence inventée");
        match &nodes[0] {
            Node::AgentAct { evidence_refs, model, .. } => {
                assert!(evidence_refs.is_empty(), "on n'invente pas une évidence absente");
                assert_eq!(model, "mode_c", "sans identité de modèle, on ne devine pas");
            }
            other => panic!("attendu un acte, obtenu {other:?}"),
        }
    }
}
