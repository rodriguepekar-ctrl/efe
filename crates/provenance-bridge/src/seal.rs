// SPDX-License-Identifier: UNLICENSED
//! **Le scellement du bundle** — P.2 : les nœuds dérivés deviennent un DAG scellé, exportable, et
//! re-prouvable par un vérifieur dont l'auditeur peut lire la totalité de la source.
//!
//! # La clé de la pile B, et pourquoi elle n'est pas celle de la pile A
//!
//! Les deux piles scellent avec un HMAC par tenant, mais **jamais avec la même clé**. Réutiliser
//! les mêmes octets à travers deux schémas de scellement distincts est précisément la classe
//! d'erreur qui casse les deux à la fois : une faiblesse trouvée dans l'un se transporte
//! immédiatement dans l'autre, et un oracle de vérification sur l'un devient un oracle sur l'autre.
//!
//! La pile B possède déjà, pour cela, son **propre contexte versionné**
//! (`provenance_dag::key`, `b"efe-prov-dag:v1:tenant:"`). [`stack_b_key`] s'en sert en lui donnant
//! la clé de la pile A comme **secret racine**. Le résultat :
//!
//! - les octets sont **différents** de ceux de la pile A — l'interdit du §2(d) est tenu ;
//! - la dérivation est **à sens unique** : détenir la clé B ne permet pas de retrouver la clé A,
//!   donc confier un bundle à un auditeur ne lui livre pas le pouvoir de forger un reçu de la
//!   pile A ;
//! - elle est **déterministe** et ne demande **aucun nouveau secret à garder** — ce qui compte,
//!   la garde des clés étant déjà une question ouverte du fil (§11.3) qu'il serait malvenu
//!   d'aggraver dès P.2 ;
//! - elle passe par `TenantKey::derive`, **source unique de vérité** : le contexte n'est jamais
//!   recopié ici (la leçon du garde de souveraineté dupliqué).
//!
//! # Ce que le bundle N'EMPORTE PAS
//!
//! Ni la clé, ni les sorties. La clé voyage **hors bande** vers l'auditeur ; le bundle ne contient
//! que des empreintes, préfixées `blake3:` pour qu'aucune ne puisse passer pour une valeur.

use provenance_dag::{DagError, Node, SealedDag, TenantKey};

use crate::derive::nodes_from_run;
use crate::error::DeriveError;
use crate::view::RunView;

/// Un bundle scellé, prêt à être remis à un auditeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// Les records du DAG, une ligne JSON par nœud — le format que lit `verify-mini`.
    pub jsonl: String,
    /// L'engagement sur la totalité du bundle. C'est **cette** valeur que P.4 publiera hors
    /// bundle, et sans laquelle une troncature reste indétectable (le bundle ne prouverait sa
    /// cohérence que contre lui-même).
    pub manifest_root: String,
    /// Le nombre de nœuds. Second garde contre la troncature, indépendant de la racine : un
    /// bundle amputé trahit **et** par sa racine **et** par son compte.
    pub node_count: u32,
}

/// Pourquoi un bundle n'a pas pu être produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// La dérivation a refusé — voir [`DeriveError`]. Le cas dominant : le run ne reproduit pas
    /// sa racine scellée, et lui donner un bundle re-prouvable le **blanchirait**.
    Derive(DeriveError),
    /// La pile B a rejeté un nœud (causalité, unicité). Un refus ici signale un défaut du
    /// **mapping**, pas du run : le pont a produit quelque chose que la pile B n'accepte pas.
    Seal(DagError),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Derive(e) => write!(f, "dérivation refusée : {e}"),
            BundleError::Seal(e) => write!(f, "la pile B a rejeté un nœud dérivé : {e:?}"),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<DeriveError> for BundleError {
    fn from(e: DeriveError) -> Self {
        BundleError::Derive(e)
    }
}

/// Dérive la clé de la **pile B** pour un tenant, depuis la clé de la **pile A**.
///
/// Voir l'en-tête du module : séparation de domaine stricte, sens unique, aucun secret neuf.
#[must_use]
pub fn stack_b_key(stack_a_key: &[u8], tenant: &str) -> TenantKey {
    TenantKey::derive(stack_a_key, tenant)
}

/// Scelle des nœuds déjà dérivés.
///
/// L'ordre des nœuds est celui que [`nodes_from_run`] a produit — il porte la causalité, et le
/// changer changerait la racine. On l'ajoute donc tel quel, sans re-trier : re-trier ici
/// introduirait un second ordre canonique, et deux ordres divergents produisent deux racines pour
/// un même run.
pub fn seal_bundle(nodes: Vec<Node>, key: &TenantKey) -> Result<Bundle, DagError> {
    let node_count = u32::try_from(nodes.len()).unwrap_or(u32::MAX);
    let mut dag = SealedDag::new();
    for n in nodes {
        dag.append(key, n)?;
    }
    Ok(Bundle { jsonl: dag.to_jsonl(), manifest_root: dag.manifest_root(), node_count })
}

/// Le chemin en un appel : d'un run scellé de la pile A au bundle de la pile B.
///
/// `stack_a_key` sert **deux fois**, et c'est voulu : à re-prouver la complétude du run (la
/// dérivation refuse si la racine scellée ne se reproduit pas), puis à dériver la clé de la pile B.
/// L'appelant n'a donc qu'un secret à manipuler, et aucune occasion de les confondre.
pub fn bundle_from_run(view: &RunView<'_>, stack_a_key: &[u8]) -> Result<Bundle, BundleError> {
    let nodes = nodes_from_run(view, stack_a_key)?;
    let key = stack_b_key(stack_a_key, view.tenant);
    seal_bundle(nodes, &key).map_err(BundleError::Seal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{AgentId, Blake3Hash, SwarmId, TenantId};
    use efe_swarm_provenance::{
        AgentCallDraft, AgentCallRecord, AgentKind, AgentRole, SwarmChain,
    };
    use provenance_dag::{hmac_sha256, reprove_dag, verify_against_root};
    use std::time::{Duration, UNIX_EPOCH};
    use uuid::Uuid;

    const TENANT: &str = "acme-corp";

    fn a_key() -> Vec<u8> {
        vec![7u8; 32]
    }

    /// Un run RÉEL de `n` actes chaînés causalement.
    fn chained_run(n: usize) -> (Uuid, Vec<AgentCallRecord>, String) {
        let tenant = TenantId::system();
        let swarm = SwarmId::new();
        let mut chain = SwarmChain::new(tenant, swarm, a_key());
        let mut drafts = Vec::new();
        let mut prev: Option<Uuid> = None;
        for i in 0..n {
            let rid = Uuid::from_u128(0xC000_0000_0000_0000_0000_0000_0000_0000u128 + i as u128);
            let mut d = AgentCallDraft::new(
                swarm,
                tenant,
                AgentId::new(format!("agent{i}")).unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                Blake3Hash::from_bytes([i as u8; 32]),
                Blake3Hash::from_bytes([(i + 60) as u8; 32]),
            )
            .with_record_id(rid)
            .with_wall_at(UNIX_EPOCH + Duration::from_millis(2_000 + i as u64))
            .wrote(format!("c{i}"), 1);
            if let Some(p) = prev {
                d = d.caused_by(p);
            }
            prev = Some(rid);
            drafts.push(d);
        }
        let records = chain.append_all_canonical(drafts);
        (Uuid::from_u128(0xF00D), records, chain.root_hex().expect("chaîne non vide"))
    }

    /// Les octets de la clé B, recalculés **indépendamment** avec le contexte publié de la pile B.
    ///
    /// C'est un **contre-contrôle**, pas une seconde source de vérité : le test
    /// `stack_b_key_agrees_with_an_independent_recomputation` prouve que ces octets sont bien ceux
    /// que `TenantKey::derive` produit. Ils servent ensuite à alimenter `verify-mini`, qui prend
    /// une clé brute.
    fn b_key_bytes(stack_a_key: &[u8], tenant: &str) -> [u8; 32] {
        let mut ctx = b"efe-prov-dag:v1:tenant:".to_vec();
        ctx.extend_from_slice(tenant.as_bytes());
        hmac_sha256(stack_a_key, &ctx)
    }

    // ── La clé ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_stack_b_key_is_never_the_stack_a_key() {
        // L'interdit du §2(d), rendu exécutable.
        let a = a_key();
        let b = b_key_bytes(&a, TENANT);
        assert_ne!(&b[..], &a[..], "les octets de la pile B ne sont JAMAIS ceux de la pile A");
    }

    #[test]
    fn stack_b_key_agrees_with_an_independent_recomputation() {
        // Prouve que le contre-contrôle ci-dessus reproduit bien `TenantKey::derive` — donc que
        // les octets donnés à verify-mini sont les bons, sans jamais extraire une clé.
        let a = a_key();
        let derived = stack_b_key(&a, TENANT);
        let rebuilt = TenantKey::from_bytes(b_key_bytes(&a, TENANT));
        let (run, recs, root) = chained_run(2);
        let nodes = nodes_from_run(&RunView::new(run, TENANT, &recs, &[], Some(&root)), &a).unwrap();
        let sealed = seal_bundle(nodes, &derived).unwrap();
        // Si les deux clés diffèrent, la re-preuve échoue.
        let records = SealedDag::records_from_jsonl(&sealed.jsonl).unwrap();
        assert!(reprove_dag(&records, &rebuilt).ok, "les deux dérivations coïncident");
    }

    #[test]
    fn the_stack_b_key_is_per_tenant() {
        let a = a_key();
        assert_ne!(b_key_bytes(&a, "acme"), b_key_bytes(&a, "globex"), "isolation tenant");
    }

    // ── Le bundle ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_sealed_bundle_reproves_and_is_deterministic() {
        let (run, recs, root) = chained_run(4);
        let view = RunView::new(run, TENANT, &recs, &[], Some(&root));
        let b1 = bundle_from_run(&view, &a_key()).expect("run sain");
        let b2 = bundle_from_run(&view, &a_key()).expect("run sain");
        assert_eq!(b1, b2, "même run ⟹ même bundle, à l'octet (donc même racine)");
        assert_eq!(b1.node_count, 7, "4 actes + 3 arêtes");

        let key = stack_b_key(&a_key(), TENANT);
        let records = SealedDag::records_from_jsonl(&b1.jsonl).expect("JSONL relisible");
        let v = verify_against_root(&records, &key, &b1.manifest_root);
        assert!(v.ok, "le bundle se re-prouve contre sa propre racine : {:?}", v.reason);
    }

    #[test]
    fn tampering_with_a_bundle_node_breaks_the_reproof_and_localises_it() {
        let (run, recs, root) = chained_run(3);
        let b = bundle_from_run(&RunView::new(run, TENANT, &recs, &[], Some(&root)), &a_key())
            .unwrap();

        // On maquille une empreinte DANS le bundle — l'attaque que le sceau doit voir.
        let tampered = b.jsonl.replacen("blake3:", "blake3:ff", 1);
        assert_ne!(tampered, b.jsonl, "le bundle a bien été modifié");

        let key = stack_b_key(&a_key(), TENANT);
        let records = SealedDag::records_from_jsonl(&tampered).expect("toujours du JSONL valide");
        let v = reprove_dag(&records, &key);
        assert!(!v.ok, "une falsification casse la re-preuve");
        assert!(v.broken_at.is_some(), "…et elle est LOCALISÉE");
    }

    #[test]
    fn truncating_a_bundle_is_caught_by_the_published_root() {
        // Le bundle nu re-prouve après troncature (un préfixe de chaîne valide l'est) : c'est
        // exactement pourquoi P.4 publiera la racine HORS du bundle.
        let (run, recs, root) = chained_run(4);
        let b = bundle_from_run(&RunView::new(run, TENANT, &recs, &[], Some(&root)), &a_key())
            .unwrap();
        let key = stack_b_key(&a_key(), TENANT);

        let mut lines: Vec<&str> = b.jsonl.lines().collect();
        lines.pop();
        let short = format!("{}\n", lines.join("\n"));
        let records = SealedDag::records_from_jsonl(&short).unwrap();

        assert!(reprove_dag(&records, &key).ok, "le bundle tronqué est intègre — le piège");
        let v = verify_against_root(&records, &key, &b.manifest_root);
        assert!(!v.ok, "…mais la racine PUBLIÉE l'attrape");
    }

    #[test]
    fn a_wrong_tenant_key_cannot_reprove_the_bundle() {
        let (run, recs, root) = chained_run(2);
        let b = bundle_from_run(&RunView::new(run, TENANT, &recs, &[], Some(&root)), &a_key())
            .unwrap();
        let other = stack_b_key(&a_key(), "globex");
        let records = SealedDag::records_from_jsonl(&b.jsonl).unwrap();
        assert!(!reprove_dag(&records, &other).ok, "isolation tenant sur la re-preuve");
    }

    #[test]
    fn a_refused_run_yields_no_bundle_at_all() {
        let (run, recs, root) = chained_run(4);
        let err = bundle_from_run(&RunView::new(run, TENANT, &recs[..3], &[], Some(&root)), &a_key())
            .expect_err("un run tronqué ne produit AUCUN bundle");
        assert!(matches!(err, BundleError::Derive(DeriveError::RootMismatch { .. })));
    }

    // ── LE test du jalon : re-preuve par verify-mini ───────────────────────────────────────

    #[test]
    fn the_bundle_reproves_under_verify_mini_the_zero_dependency_verifier() {
        // Le critère d'acceptation de P.2, et la démonstration entière du fil : le bundle produit
        // par la pile A se re-prouve sous un binaire dont l'arbre de dépendances est LITTÉRALEMENT
        // vide — donc dont un auditeur peut lire la totalité de la source avant de le croire.
        // `verify-mini` est ici une DEV-dépendance : il n'entre pas dans l'arbre normal du pont,
        // et l'arbre du vérifieur reste vide (patron déjà utilisé pour le contre-contrôle S0).
        let (run, recs, root) = chained_run(5);
        let b = bundle_from_run(&RunView::new(run, TENANT, &recs, &[], Some(&root)), &a_key())
            .expect("run sain");
        let key = b_key_bytes(&a_key(), TENANT);

        let verdict = verify_mini::run(&b.jsonl, &key, Some(&b.manifest_root))
            .expect("verify-mini relit le JSONL du pont");
        assert!(verdict.ok, "re-prouvé par le vérifieur à dépendances vides");
        assert_eq!(verdict.n_records as u32, b.node_count, "même compte de nœuds");

        // …et il refuse une racine qui n'est pas la sienne.
        let bad = verify_mini::run(&b.jsonl, &key, Some(&"ff".repeat(32))).unwrap();
        assert!(!bad.ok, "verify-mini confronte bien la racine publiée");
    }
}
