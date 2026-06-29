// SPDX-License-Identifier: UNLICENSED
//! **Les gabarits** — les registres de mapping curatés, par cadre. C'est le **packaging réglementaire**
//! neuf de P1 (§7) : des **revendications déclarées, datées, versionnées** — pas du code enfoui, pas une
//! vérité. Chaque gabarit est **lisible**, **contestable**, et **honnête sur ses trous** (chaque cadre
//! déclare au moins un [`Coverage::OutOfDagScope`], sinon [`MappingSet::new`] le rejette).
//!
//! **Note d'honnêteté (rappel).** Les références (EU AI Act 2024/1689 ; GDPR 2016/679 ; CSDDD 2024/1760 ;
//! LkSG) sont des **cibles d'outillage**, pas un avis juridique : l'applicabilité (classification « haut
//! risque », articles, conformité finale) dépend de la classification du système et du conseil du client.
//! Ces gabarits **mappent l'évidence** ; ils ne **prononcent** rien (`outillé ≠ conforme`).

use crate::framework::{Coverage, Framework};
use crate::mapping::{MappingSet, RequirementMapping};
use crate::selector::{Field, NodeKind, Selector};
use provenance_dag::HumanAction;

/// La version et la date de publication communes des gabarits (figées — pas d'horloge).
const VERSION: &str = "2026-06";
const PUBLISHED_AT: &str = "2026-06-29";

/// Raccourci de construction d'une maille. `claim = (furnishes, does_not_furnish)` est groupé en un tuple
/// (les **deux faces** d'une même revendication : ce que le DAG fournit, et le trou honnête).
fn req(framework: Framework, citation: &str, requirement_text: &str, coverage: Coverage, selector: Option<Selector>, claim: (&str, &str)) -> RequirementMapping {
    RequirementMapping {
        framework,
        citation: citation.to_string(),
        requirement_text: requirement_text.to_string(),
        coverage,
        selector,
        furnishes: claim.0.to_string(),
        does_not_furnish: claim.1.to_string(),
    }
}

/// Sélecteur « tout le journal scellé » (les quatre types de nœuds) — l'évidence d'une tenue de registres.
fn whole_log() -> Selector {
    Selector::Any(vec![
        Selector::Kind(NodeKind::AgentAct),
        Selector::Kind(NodeKind::Edge),
        Selector::Kind(NodeKind::Evidence),
        Selector::Kind(NodeKind::Human),
    ])
}

/// Sélecteur « l'oversight humain actif » (Approve/Override/Escalate — pas un simple commentaire).
fn active_oversight() -> Selector {
    Selector::All(vec![
        Selector::Kind(NodeKind::Human),
        Selector::HumanActionIn(vec![HumanAction::Approve, HumanAction::Override, HumanAction::Escalate]),
    ])
}

/// **Gabarit EU AI Act** — Règlement (UE) 2024/1689, régime haut risque (cf. architecture S6 §6).
///
/// Le cadre où le DAG marque le plus de points (la tenue de registres de l'art. 12 est le *home run* :
/// le DAG EST le registre, et il est **infalsifiable** — au-delà de la lettre). Les trous (art. 9/10/11/
/// 17/72) sont **déclarés** : ce sont des artefacts organisationnels que le DAG ne produit pas.
pub fn eu_ai_act_2026_06() -> MappingSet {
    let m = vec![
        req(Framework::EuAiAct, "art. 12", "Tenue de registres : enregistrement automatique des événements (logs) sur la durée de vie, traçabilité appropriée.", Coverage::Covered, Some(whole_log()),
            ("le DAG scellé EST le registre d'événements automatique — et il est tamper-evident (chaque acte, arête, évidence, et nœud humain, horodaté et chaîné) : au-delà de la lettre de l'exigence.",
             "l'art. 12 vise une capacité de journalisation ; la politique de rétention/conservation reste une décision du déployeur.")),
        req(Framework::EuAiAct, "art. 13", "Transparence & information des déployeurs : interpréter la sortie, l'utiliser correctement.", Coverage::Partial, Some(Selector::Kind(NodeKind::Evidence)),
            ("la couche d'évidence (clause/document cités, content_hash figé au moment T, lien de consommation) reconstruit la chaîne de raisonnement d'une sortie donnée → interprétabilité a posteriori.",
             "l'art. 13 inclut aussi la notice d'utilisation et l'information de conception fournies par le fournisseur — documents organisationnels hors DAG.")),
        req(Framework::EuAiAct, "art. 14", "Contrôle humain : système conçu pour être effectivement supervisé par des personnes physiques.", Coverage::Partial, Some(active_oversight()),
            ("les nœuds humains scellés (APPROVE/OVERRIDE/ESCALATE, cible, justification, horodatage) prouvent que l'oversight a eu lieu, par qui, et sur quelle décision.",
             "prouve que le contrôle a eu lieu et fut enregistré — PAS que le dispositif d'oversight est « effectif » au sens de l'art. 14 (cela relève de la conception et de l'organisation).")),
        req(Framework::EuAiAct, "art. 26", "Obligations du déployeur : assurer l'oversight, conserver les logs auto-générés (sous son contrôle).", Coverage::Covered, Some(Selector::Any(vec![Selector::Kind(NodeKind::Human), Selector::Kind(NodeKind::AgentAct)])),
            ("le bundle démontre, re-prouvablement et exportablement, que le déployeur a tenu les logs auto-générés et assuré l'oversight.",
             "l'art. 26 porte d'autres devoirs (usage conforme à la notice, surveillance, information des personnes concernées) que le seul DAG n'établit pas.")),
        // ── les TROUS, déclarés (anti-vert-malhonnête, §5) ──
        req(Framework::EuAiAct, "art. 9", "Système de gestion des risques (sur le cycle de vie).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : artefact organisationnel et de cycle de vie, maintenu ailleurs par le déployeur/fournisseur.")),
        req(Framework::EuAiAct, "art. 10", "Gouvernance des données (qualité, biais des jeux d'entraînement/validation).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : propriété des données et du processus d'entraînement, pas de l'exécution auditée.")),
        req(Framework::EuAiAct, "art. 11 + Annexe IV", "Documentation technique du système.", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : dossier technique organisationnel (conception, performances, conformité) maintenu hors exécution.")),
        req(Framework::EuAiAct, "art. 17", "Système de gestion de la qualité.", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : dispositif qualité organisationnel du fournisseur.")),
        req(Framework::EuAiAct, "art. 72", "Surveillance après commercialisation.", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : processus post-marché organisationnel ; le DAG en serait une source, pas le système.")),
    ];
    MappingSet::new(VERSION, PUBLISHED_AT, m).expect("gabarit EU AI Act honnête (un trou déclaré)")
}

/// **Gabarit GDPR** — Règlement (UE) 2016/679 (cf. architecture S6 §7).
///
/// Honnêteté particulièrement nette : le GDPR mêle des exigences **techniques** (que le DAG touche) et de
/// **base légale** (que le DAG n'établit jamais). L'articulation append-only ↔ droit à l'effacement
/// (art. 17) est nommée comme la voie **crypto-shredding** (raffinement coordonné S0/S1, hors S6).
pub fn gdpr_2026_06() -> MappingSet {
    let m = vec![
        req(Framework::Gdpr, "art. 5(1)(c)", "Minimisation des données : adéquates, pertinentes, limitées au nécessaire.", Coverage::Partial, Some(Selector::All(vec![Selector::Kind(NodeKind::AgentAct), Selector::FieldPresent(Field::InputsHash)])),
            ("la capture scelle des HACHAGES d'entrée (jamais le brut) → minimisation native dans la piste d'audit : on prouve ce qui a été décidé sans retenir la PII brute dans le record.",
             "la minimisation est une propriété du design de capture ; le client contrôle ce qui entre dans les prompts en amont ; un hash n'est ni une base légale ni une anonymisation au sens du droit.")),
        req(Framework::Gdpr, "art. 22", "Décision individuelle automatisée : droit d'obtenir une intervention humaine, garde-fous.", Coverage::Partial, Some(active_oversight()),
            ("les nœuds humains + la chaîne de décision évidencient que la garde d'intervention humaine existe et a opéré, traçablement.",
             "l'art. 22 porte sur la base légale et l'EXISTENCE de la garde ; le DAG évidence l'intervention, il n'établit pas le fondement juridique du traitement automatisé.")),
        req(Framework::Gdpr, "art. 30", "Registre des activités de traitement.", Coverage::Partial, Some(Selector::Any(vec![Selector::Kind(NodeKind::AgentAct), Selector::Kind(NodeKind::Edge)])),
            ("le DAG est un registre de traitement granulaire et vérifiable (qui a traité quoi, quand, sur quelle évidence).",
             "complément, pas substitut, du registre art. 30 (catégories de personnes, destinataires, transferts, durées) qui reste un document organisationnel/légal.")),
        // ── le TROU, déclaré ──
        req(Framework::Gdpr, "art. 6 / 35 / 15-17", "Base légale ; analyse d'impact (DPIA) ; droits des personnes (accès/effacement).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : la licéité (art. 6), l'analyse d'impact (art. 35) et l'exercice des droits sont juridiques/organisationnels. L'articulation de l'append-only avec le droit à l'effacement (art. 17) est le crypto-shredding (raffinement nommé coordonné S0/S1, hors S6) — pas une « compatibilité effacement » du DAG tel quel.")),
    ];
    MappingSet::new(VERSION, PUBLISHED_AT, m).expect("gabarit GDPR honnête (un trou déclaré)")
}

/// **Gabarit devoir de vigilance** — CSDDD (Directive (UE) 2024/1760) **et** LkSG (cf. architecture S6 §8).
///
/// Le cadre qui porte le **ROI** (le dossier fournisseur, généré, re-prouvable, défendable). Deux cadres
/// distincts (`Csddd` et `Lksg`) coexistent dans un même registre — **chacun** déclare son trou, donc
/// `derive(…, Framework::Csddd, …)` produit le dossier CSDDD et `derive(…, Framework::Lksg, …)` le LkSG.
pub fn csddd_lksg_2026_06() -> MappingSet {
    let m = vec![
        // ── CSDDD (Directive (UE) 2024/1760) ──
        req(Framework::Csddd, "art. 8", "Identification des impacts négatifs (droits humains / environnement) dans la chaîne d'activités.", Coverage::Partial, Some(Selector::Kind(NodeKind::Evidence)),
            ("les évidences consultées (criblage, critères ESG) sont figées au moment T (content_hash) — la version réellement utilisée pour identifier le risque.",
             "la méthodologie complète d'analyse de risque (cartographie, pondération) reste organisationnelle.")),
        req(Framework::Csddd, "art. 10", "Mesures de prévention/atténuation des impacts.", Coverage::Covered, Some(Selector::Kind(NodeKind::AgentAct)),
            ("les contrôles exécutés (criblage sanctions, vérification ESG) sont attestés, avec leur conclusion et le modèle qui a conclu.",
             "la remédiation terrain et l'engagement contractuel des partenaires ne sont pas dans le DAG.")),
        req(Framework::Csddd, "art. 11", "Documentation et communication des mesures de diligence.", Coverage::Covered, Some(whole_log()),
            ("le dossier re-prouvable EST la documentation de diligence — chaque ligne tracée à un nœud scellé, exportable au régulateur.",
             "la communication publique (reporting de durabilité) reste une obligation organisationnelle distincte.")),
        req(Framework::Csddd, "art. 7 / 14", "Politique de diligence intégrée ; mécanisme de notification/plainte.", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : politique-cadre et mécanisme de plainte — dispositifs organisationnels.")),
        // ── LkSG (Lieferkettensorgfaltspflichtengesetz) ──
        req(Framework::Lksg, "§6", "Mesures de prévention (Präventionsmaßnahmen) chez le fournisseur.", Coverage::Covered, Some(Selector::Kind(NodeKind::AgentAct)),
            ("les contrôles fournisseurs exécutés sont attestés (la mesure réellement appliquée, pas une déclaration).",
             "les mesures préventives organisationnelles (formation, clauses contractuelles, audits terrain) restent hors DAG.")),
        req(Framework::Lksg, "§10(1)", "Documentation continue de la diligence (Dokumentation).", Coverage::Covered, Some(whole_log()),
            ("la documentation continue, granulaire et re-prouvable, de la diligence — la conservation pluriannuelle exigée est facilitée par un journal scellé.",
             "la durée de conservation (7 ans) est une obligation de rétention du déployeur, pas une propriété du DAG.")),
        req(Framework::Lksg, "revue", "Revue / décision de supervision humaine (oversight).", Coverage::Covered, Some(active_oversight()),
            ("la revue/approbation/escalade humaine est scellée (qui, quand, sur quelle décision) — l'oversight démontré.",
             "l'adéquation du dispositif de revue (compétence, indépendance) reste organisationnelle.")),
        req(Framework::Lksg, "§8 / §9", "Mesures correctives (Abhilfemaßnahmen) ; procédure de plainte (Beschwerdeverfahren).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : mesures correctives terrain et procédure de plainte — dispositifs organisationnels.")),
    ];
    MappingSet::new(VERSION, PUBLISHED_AT, m).expect("gabarit CSDDD/LkSG honnête (un trou par cadre)")
}

/// Les **trois gabarits** de la version courante (commodité : tout charger d'un coup).
pub fn all_2026_06() -> Vec<MappingSet> {
    vec![eu_ai_act_2026_06(), gdpr_2026_06(), csddd_lksg_2026_06()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_templates_construct_and_are_honest() {
        // I-S6-COUVERTURE-HONNÊTE au niveau gabarit : chacun construit (donc chaque cadre a un trou), et
        // déclare effectivement ≥ 1 hors-périmètre par cadre présent.
        for set in all_2026_06() {
            for fw in set.frameworks() {
                let reqs = set.for_framework(fw);
                assert!(reqs.iter().any(|m| m.coverage == Coverage::OutOfDagScope), "{} déclare au moins un trou", fw.label());
                assert!(reqs.iter().any(|m| m.coverage != Coverage::OutOfDagScope), "{} fournit aussi de l'évidence", fw.label());
            }
        }
    }

    #[test]
    fn eu_ai_act_has_the_record_keeping_home_run_and_declared_holes() {
        let set = eu_ai_act_2026_06();
        let art12 = set.for_framework(Framework::EuAiAct).into_iter().find(|m| m.citation == "art. 12").unwrap();
        assert_eq!(art12.coverage, Coverage::Covered, "l'art. 12 est le home run (couvert)");
        // Les trous attendus sont déclarés.
        let holes: Vec<&str> = set.for_framework(Framework::EuAiAct).into_iter().filter(|m| m.coverage == Coverage::OutOfDagScope).map(|m| m.citation.as_str()).collect();
        for expected in ["art. 9", "art. 10", "art. 11 + Annexe IV", "art. 17", "art. 72"] {
            assert!(holes.contains(&expected), "trou déclaré : {expected}");
        }
    }

    #[test]
    fn csddd_and_lksg_are_two_frameworks_each_with_a_hole() {
        let set = csddd_lksg_2026_06();
        assert_eq!(set.frameworks(), vec![Framework::Csddd, Framework::Lksg]);
        for fw in [Framework::Csddd, Framework::Lksg] {
            assert!(set.for_framework(fw).iter().any(|m| m.coverage == Coverage::OutOfDagScope), "{} a un trou", fw.label());
        }
    }

    #[test]
    fn gdpr_names_the_erasure_tension_as_crypto_shredding() {
        // L'honnêteté GDPR doit NOMMER le crypto-shredding (l'articulation append-only ↔ art. 17), pas
        // prétendre que l'audit scellé est « compatible effacement » tel quel.
        let set = gdpr_2026_06();
        let hole = set.for_framework(Framework::Gdpr).into_iter().find(|m| m.coverage == Coverage::OutOfDagScope).unwrap();
        assert!(hole.does_not_furnish.contains("crypto-shredding"), "le crypto-shredding est nommé");
        assert!(hole.does_not_furnish.contains("art. 17"), "l'articulation avec l'art. 17 est nommée");
    }

    #[test]
    fn template_set_hash_is_stable_across_calls() {
        // Deux constructions du même gabarit → même hash (les gabarits sont des constantes, pas une horloge).
        assert_eq!(eu_ai_act_2026_06().set_hash(), eu_ai_act_2026_06().set_hash());
        // Les trois gabarits ont des hash distincts (contenus distincts).
        let hs: Vec<String> = all_2026_06().iter().map(|s| s.set_hash()).collect();
        let mut uniq = hs.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), hs.len(), "trois gabarits distincts → trois hash distincts");
    }
}
