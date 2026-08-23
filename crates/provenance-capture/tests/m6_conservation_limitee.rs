// SPDX-License-Identifier: UNLICENSED
//! # M6 — le critère, exécuté : « un cas expiré disparaît, et tout le reste se re-prouve encore ».
//!
//! Le critère a l'air contradictoire. Une chaîne scellée se replie sur les **octets entiers** de
//! chaque nœud (`chaîne ← HMAC(clé, chaîne ‖ canonique(nœud))`) : retirer un record casse son sceau,
//! tous les suivants, et les références qui pointaient vers lui.
//!
//! Il ne l'est pas, parce qu'on ne retire rien. Les champs effaçables sont scellés **chiffrés** ; le
//! broyage détruit la clé du cas. Ce fichier le démontre dans la forme la plus dure qui soit : non
//! pas « le DAG se re-prouve encore », mais **son JSONL est octet pour octet le même**.

use provenance_capture::conservation::{enrober, lire, Conservation};
use provenance_dag::shred::{CaseKey, Lecture, Trousseau};
use provenance_dag::{est_enveloppe, reprove_dag, HumanAction, Node, SealedDag, TenantKey};

/// Les trois cas de la démonstration : deux vieux, un récent.
const CAS: [(&str, u64, &str, &str); 3] = [
    ("DOSSIER-A", 0, "COMPLIANT", "revue faite, pièces au dossier"),
    ("DOSSIER-B", 100, "PASS", "aucune correspondance sanction"),
    ("DOSSIER-C", 900, "ESCALADE", "second avis demandé au comité"),
];

fn cle_de(cas: &str) -> CaseKey {
    // ⚠ Une VRAIE clé de cas est ALÉATOIRE. Ici on la fabrique pour que le test soit reproductible ;
    // dériver en production annulerait le broyage (voir `provenance_dag::shred`).
    let mut b = [0u8; 32];
    for (i, o) in cas.bytes().enumerate().take(32) {
        b[i] = o;
    }
    CaseKey::from_bytes(b)
}

/// Construit le DAG des trois cas, tous champs effaçables enrobés, et le trousseau qui va avec.
fn scene() -> (SealedDag, Trousseau, TenantKey) {
    let k = TenantKey::derive(b"racine-de-test-m6", "acme");
    let mut dag = SealedDag::new();
    let mut trousseau = Trousseau::new();

    for (cas, ouvert_a, sortie, motif) in CAS {
        let cle = cle_de(cas);
        assert!(trousseau.poser(cas, cle_de(cas), ouvert_a));

        let acte = Node::AgentAct {
            node_id: format!("{cas}::verdict"),
            agent_role: "verdict_aggregate".into(),
            inputs_hash: "0".repeat(64),
            model: "phi4".into(),
            version: "1".into(),
            output: sortie.into(),
            evidence_refs: vec![],
            timestamp: ouvert_a,
        };
        dag.append(&k, enrober(acte, &cle)).expect("acte");

        let revue = Node::Human {
            node_id: format!("{cas}::revue"),
            human_id: format!("agent-{cas}@exemple"),
            action: HumanAction::Approve,
            target_node: format!("{cas}::verdict"),
            justification: motif.into(),
            timestamp: ouvert_a + 1,
        };
        dag.append(&k, enrober(revue, &cle)).expect("revue");
    }
    (dag, trousseau, k)
}

#[test]
fn m6_un_cas_expire_disparait_et_tout_le_reste_se_reprouve_encore() {
    let (dag, mut trousseau, k) = scene();
    let records = dag.records().to_vec();

    // ── Avant. La chaîne tient, et les trois cas se lisent. ──────────────────────────────────
    let avant = reprove_dag(&records, &k);
    assert!(avant.ok, "le DAG doit se prouver avant tout broyage");
    let racine_avant = avant.manifest_root.clone().expect("racine");
    let jsonl_avant = dag.to_jsonl();

    for (cas, _, sortie, motif) in CAS {
        let vue = lire(&records[cas_index(cas) * 2].node, &trousseau);
        assert_eq!(vue["output"], Lecture::Ouvert(sortie.into()), "{cas} : lisible avant");
        let vue_h = lire(&records[cas_index(cas) * 2 + 1].node, &trousseau);
        assert_eq!(vue_h["justification"], Lecture::Ouvert(motif.into()));
    }

    // ── La politique passe. Horizon 500 s, il est 1000 : A et B ont expiré, C non. ────────────
    let balayage = Conservation::nouvelle(500).appliquer(&mut trousseau, 1000);
    assert_eq!(
        balayage.broyes,
        vec!["DOSSIER-A".to_string(), "DOSSIER-B".to_string()],
        "seuls les expirés"
    );
    assert_eq!((balayage.vivants, balayage.total_broyes), (1, 2), "2 sur 3 — avec son dénominateur");

    // ── « un cas expiré DISPARAÎT » ──────────────────────────────────────────────────────────
    for cas in ["DOSSIER-A", "DOSSIER-B"] {
        let acte = &records[cas_index(cas) * 2].node;
        let vue = lire(acte, &trousseau);
        assert_eq!(
            vue["output"],
            Lecture::Broye { le: 1000 },
            "{cas} : le contenu est détruit, et la DATE de destruction demeure"
        );
        let vue_h = lire(&records[cas_index(cas) * 2 + 1].node, &trousseau);
        assert_eq!(vue_h["human_id"], Lecture::Broye { le: 1000 }, "{cas} : la personne aussi");
        assert_eq!(vue_h["justification"], Lecture::Broye { le: 1000 });

        // La TRACE reste — c'est ce qui permet à la chaîne de tenir, et c'est ce qu'il faut dire :
        // un auditeur voit toujours qu'un acte a eu lieu ; il ne peut plus lire ce qu'il disait.
        let Node::AgentAct { node_id, output, agent_role, timestamp, .. } = acte else { panic!() };
        assert_eq!(node_id, &format!("{cas}::verdict"));
        assert_eq!(agent_role, "verdict_aggregate");
        assert!(est_enveloppe(output), "le chiffré demeure, reconnaissable — pas un champ vide");
        assert!(*timestamp < 1000);
    }

    // ── « et TOUT LE RESTE SE RE-PROUVE ENCORE » ─────────────────────────────────────────────
    let apres = reprove_dag(&records, &k);
    assert!(apres.ok, "la chaîne tient après le broyage");
    assert_eq!(
        apres.manifest_root.as_ref(),
        Some(&racine_avant),
        "MÊME manifest-root : le broyage n'a pas déplacé la chaîne d'un cran"
    );

    // La forme la plus dure du même énoncé : pas un octet n'a bougé.
    assert_eq!(dag.to_jsonl(), jsonl_avant, "le DAG est octet pour octet identique");

    // ── Et le cas non expiré n'a rien perdu. ─────────────────────────────────────────────────
    let vue = lire(&records[cas_index("DOSSIER-C") * 2].node, &trousseau);
    assert_eq!(vue["output"], Lecture::Ouvert("ESCALADE".into()), "le récent se lit encore");
}

/// Un cas jamais confié au trousseau ne se lit pas « broyé » : on ne peut pas affirmer une
/// destruction qu'on n'a pas faite. Même distinction que 410 vs 404 dans la passerelle.
#[test]
fn m6_une_cle_absente_ne_se_declare_pas_broyee() {
    let (dag, _, _) = scene();
    let vide = Trousseau::new();
    let vue = lire(&dag.records()[0].node, &vide);
    assert_eq!(vue["output"], Lecture::CleAbsente, "absence de clé ≠ destruction");
}

fn cas_index(cas: &str) -> usize {
    CAS.iter().position(|(c, ..)| *c == cas).expect("cas connu")
}
