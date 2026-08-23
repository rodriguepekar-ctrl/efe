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
//!
//! **Les versions ne marchent pas au pas.** Une version est **la lecture d'un cadre**, datée — pas un
//! numéro de publication commun. Chaque [`MappingSet`] porte donc la sienne, et un cadre ne bouge que
//! quand **sa** lecture bouge : le GDPR est en `2026-08` (v2 — l'art. 5(1)(e), cf. [`gdpr_2026_08`])
//! pendant que l'EU AI Act et le devoir de vigilance restent en `2026-06`. Re-estampiller une lecture
//! **inchangée** annoncerait un changement qui n'a pas eu lieu : c'est le mensonge symétrique de celui
//! qu'on commet en modifiant un contenu sans toucher au numéro. Les deux sont interdits ici — et
//! [`MappingSet`] le dit déjà en toutes lettres : « on n'écrase jamais une version ».

use crate::framework::{Coverage, Framework};
use crate::mapping::{MappingSet, RequirementMapping};
use crate::selector::{Field, NodeKind, Selector};
use provenance_dag::HumanAction;

/// La lecture de **juin 2026** (figée — pas d'horloge). Elle reste la lecture **courante** de l'EU AI
/// Act et du devoir de vigilance, et la lecture **historique** du GDPR (cf. [`gdpr_2026_06`]).
const VERSION: &str = "2026-06";
const PUBLISHED_AT: &str = "2026-06-29";

/// La lecture d'**août 2026** — celle du **GDPR seul**, et pour une raison de contenu précise : M6 a
/// implémenté le crypto-shredding en S0/S1, donc l'art. 5(1)(e) entre au registre et la mise en garde de
/// l'art. 6/35/15-17 cesse de sous-annoncer. Les deux autres cadres n'ont **rien** à re-déclarer.
const VERSION_GDPR_V2: &str = "2026-08";
const PUBLISHED_AT_GDPR_V2: &str = "2026-08-23";

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

/// Sélecteur « les nœuds qui portent du **contenu effaçable** » — ceux qu'un broyage (M6) vide de leur
/// substance sans les faire disparaître.
///
/// L'**arête en est absente**, et ce n'est pas un oubli : d'après l'inventaire de
/// `provenance_capture::conservation`, **tous** ses champs (`from_node`, `to_node`, `artifact_type`)
/// survivent à un broyage — les chiffrer romprait l'intégrité structurelle sans rien protéger.
fn contenu_effacable() -> Selector {
    Selector::Any(vec![
        Selector::Kind(NodeKind::AgentAct),
        Selector::Kind(NodeKind::Human),
        Selector::Kind(NodeKind::Evidence),
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

/// Les trois mailles GDPR **communes** aux deux lectures (art. 5(1)(c), 22, 30) — partagées pour que le
/// diff entre v1 et v2 soit **exactement** ce qui a changé, et rien d'autre.
///
/// ⚠ **Ce socle est gelé de fait** : il alimente une version FIGÉE ([`gdpr_2026_06`]). Le modifier
/// changerait le `set_hash` de « 2026-06 » **sans** changer son numéro — précisément le mensonge de
/// version que ce registre existe pour rendre impossible. Le test `the_frozen_gdpr_v1_is_pinned_by_hash`
/// épingle ce hash à une constante : toucher ce socle casse le build, et c'est voulu.
fn gdpr_socle() -> Vec<RequirementMapping> {
    vec![
        req(Framework::Gdpr, "art. 5(1)(c)", "Minimisation des données : adéquates, pertinentes, limitées au nécessaire.", Coverage::Partial, Some(Selector::All(vec![Selector::Kind(NodeKind::AgentAct), Selector::FieldPresent(Field::InputsHash)])),
            ("la capture scelle des HACHAGES d'entrée (jamais le brut) → minimisation native dans la piste d'audit : on prouve ce qui a été décidé sans retenir la PII brute dans le record.",
             "la minimisation est une propriété du design de capture ; le client contrôle ce qui entre dans les prompts en amont ; un hash n'est ni une base légale ni une anonymisation au sens du droit.")),
        req(Framework::Gdpr, "art. 22", "Décision individuelle automatisée : droit d'obtenir une intervention humaine, garde-fous.", Coverage::Partial, Some(active_oversight()),
            ("les nœuds humains + la chaîne de décision évidencient que la garde d'intervention humaine existe et a opéré, traçablement.",
             "l'art. 22 porte sur la base légale et l'EXISTENCE de la garde ; le DAG évidence l'intervention, il n'établit pas le fondement juridique du traitement automatisé.")),
        req(Framework::Gdpr, "art. 30", "Registre des activités de traitement.", Coverage::Partial, Some(Selector::Any(vec![Selector::Kind(NodeKind::AgentAct), Selector::Kind(NodeKind::Edge)])),
            ("le DAG est un registre de traitement granulaire et vérifiable (qui a traité quoi, quand, sur quelle évidence).",
             "complément, pas substitut, du registre art. 30 (catégories de personnes, destinataires, transferts, durées) qui reste un document organisationnel/légal.")),
    ]
}

/// **Gabarit GDPR — v1, FIGÉE (`2026-06`).** Règlement (UE) 2016/679 (cf. architecture S6 §7).
///
/// **Ne modifiez pas cette fonction.** Ce n'est plus la lecture courante — [`gdpr_2026_08`] l'est. Elle
/// existe pour qu'un dossier **déjà émis** sous « 2026-06 » se re-dérive **octet pour octet**, ce que
/// [`MappingSet`] promet en toutes lettres (« on n'écrase jamais une version : une v2 = un nouveau set »).
///
/// Sa mise en garde de l'art. 6/35/15-17 nomme le crypto-shredding comme un **raffinement coordonné
/// S0/S1** *à venir* : c'était exact à la date de figeage, où le mécanisme n'existait pas encore. M6 l'a
/// depuis implémenté — et c'est [`gdpr_2026_08`] qui le dit, parce que le dire **ici** changerait le
/// contenu d'une lecture déjà partie chez des lecteurs.
pub fn gdpr_2026_06() -> MappingSet {
    let mut m = gdpr_socle();
    // ── le TROU, déclaré ──
    m.push(req(Framework::Gdpr, "art. 6 / 35 / 15-17", "Base légale ; analyse d'impact (DPIA) ; droits des personnes (accès/effacement).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : la licéité (art. 6), l'analyse d'impact (art. 35) et l'exercice des droits sont juridiques/organisationnels. L'articulation de l'append-only avec le droit à l'effacement (art. 17) est le crypto-shredding (raffinement nommé coordonné S0/S1, hors S6) — pas une « compatibilité effacement » du DAG tel quel.")));
    MappingSet::new(VERSION, PUBLISHED_AT, m).expect("gabarit GDPR honnête (un trou déclaré)")
}

/// **Gabarit GDPR — v2, COURANTE (`2026-08`).** Règlement (UE) 2016/679 (cf. architecture S6 §7).
///
/// Honnêteté particulièrement nette : le GDPR mêle des exigences **techniques** (que le DAG touche) et de
/// **base légale** (que le DAG n'établit jamais).
///
/// ## Pourquoi une v2, et pas une retouche
///
/// M6 a implémenté le crypto-shredding en S0/S1 (`provenance_dag::shred`,
/// `provenance_capture::conservation`). Deux conséquences, toutes deux de **contenu** — donc un
/// `set_hash` neuf, donc un **numéro** neuf (`I-S6-MAPPING-VERSIONNÉ`) :
///
/// 1. l'art. **5(1)(e)** (limitation de la conservation) entre au registre, en **PARTIEL** : le DAG
///    fournit désormais le **mécanisme d'exécution** et la preuve que la chaîne y survit — **jamais** la
///    politique. L'horizon reste une décision du déployeur, exactement comme la rétention de l'art. 12
///    de l'AI Act ;
/// 2. la mise en garde de l'art. 6/35/15-17 cesse de **sous-annoncer** : le crypto-shredding n'y est plus
///    une voie *nommée*, il est **écrit** — sans que la **portée** de cette ligne ne bouge d'un pouce.
///
/// Retoucher [`gdpr_2026_06`] au lieu d'ouvrir cette v2 aurait changé son `set_hash` sous un numéro
/// inchangé : un artefact destiné à des régulateurs aurait alors menti sur sa propre lecture.
pub fn gdpr_2026_08() -> MappingSet {
    let mut m = gdpr_socle();
    // L'art. 5(1)(e) se lit contre l'art. 5(1)(c) : ce sont les deux principes de l'art. 5 que le DAG
    // touche, l'un par ce qu'il retient, l'autre par ce qu'il cesse de retenir. On les met côte à côte —
    // l'ordre déclaré ici est l'ordre que lira un régulateur dans l'artefact rendu.
    m.insert(1, req(Framework::Gdpr, "art. 5(1)(e)", "Limitation de la conservation : conservées sous une forme permettant l'identification des personnes concernées pendant une durée n'excédant pas celle nécessaire aux finalités.", Coverage::Partial, Some(contenu_effacable()),
            ("le MÉCANISME d'exécution existe et est servi (M6) : les champs effaçables (décision, justification, identifiant d'acteur, référence de document) sont scellés CHIFFRÉS par cas, et détruire la clé d'un cas (« broyer ») rend son contenu irrécupérable SANS qu'un octet du DAG ne bouge — la chaîne se re-prouve encore, et le vérifieur tiers n'a rien à changer pour le constater. Un balayage par horizon broie les cas expirés en une opération.",
             "le DAG n'applique AUCUNE politique de conservation : l'horizon et le déclenchement du balayage sont des décisions du déployeur, liées aux finalités — comme la rétention de l'art. 12 de l'AI Act. Trois limites à dire en toutes lettres. (1) La posture est OPT-IN et sans effet rétroactif : un record scellé avant l'activation du trousseau porte son contenu en clair pour toujours (le re-chiffrer changerait ses octets, donc son sceau), et un redémarrage repart sans trousseau tant que le déployeur ne l'a pas restauré. (2) Ce n'est PAS une preuve de destruction : sauvegardes et instantanés peuvent conserver la clé ailleurs, ce que cette chaîne ne peut ni voir ni démontrer à un tiers. (3) Le nœud RESTE — son identifiant, son rôle, son horodatage et ses arêtes demeurent, car c'est ce qui permet à la chaîne de tenir ; un auditeur voit toujours qu'un acte a eu lieu à telle date, mais ne peut plus lire ce qu'il disait. Si l'identifiant de cas est lui-même une donnée personnelle, le broyage ne l'atteint pas.")));
    // ── le TROU, déclaré ──
    m.push(req(Framework::Gdpr, "art. 6 / 35 / 15-17", "Base légale ; analyse d'impact (DPIA) ; droits des personnes (accès/effacement).", Coverage::OutOfDagScope, None,
            ("", "hors périmètre du DAG : la licéité (art. 6), l'analyse d'impact (art. 35) et l'exercice des droits restent juridiques/organisationnels — le DAG ne reçoit aucune demande, ne vérifie l'identité d'aucun demandeur, et ne juge du bien-fondé d'aucune. L'articulation de l'append-only avec le droit à l'effacement (art. 17) est le crypto-shredding : ce raffinement coordonné S0/S1 est désormais IMPLÉMENTÉ et servi (cf. l'art. 5(1)(e) ci-dessus, et ses trois limites), et il reste hors S6. Ce qui a changé est donc l'existence du moyen, pas la portée de cette ligne : disposer de quoi exécuter un effacement n'est ni l'instruction d'une demande, ni une « compatibilité effacement » du DAG tel quel.")));
    MappingSet::new(VERSION_GDPR_V2, PUBLISHED_AT_GDPR_V2, m).expect("gabarit GDPR v2 honnête (un trou déclaré)")
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

/// La **famille de juin 2026**, telle qu'elle fut publiée — y compris le GDPR v1, **figé**. Ce nom reste
/// vrai : les trois lectures qu'il rend portent bien toutes le numéro `2026-06`. C'est ce qu'il faut pour
/// re-dériver un dossier de cette époque ; ce n'est **pas** ce qu'il faut pour en émettre un neuf.
pub fn all_2026_06() -> Vec<MappingSet> {
    vec![eu_ai_act_2026_06(), gdpr_2026_06(), csddd_lksg_2026_06()]
}

/// Les **lectures courantes** — une par cadre, chacune à **sa** version (le GDPR en `2026-08`, les deux
/// autres en `2026-06`). C'est ce que sert la passerelle, et ce qu'un dossier neuf doit porter.
///
/// Deux fonctions plutôt qu'une parce que les deux questions sont distinctes : « sous quelle lecture ce
/// dossier-là fut-il produit ? » (historique, [`all_2026_06`]) et « sous quelle lecture produire ? »
/// (courante). Les confondre, c'est re-dériver un vieux dossier sous une règle qu'il n'a jamais connue.
pub fn all_courants() -> Vec<MappingSet> {
    vec![courant(Framework::EuAiAct), courant(Framework::Gdpr), courant(Framework::Csddd)]
}

/// **La lecture courante d'un cadre** — le seul endroit qui réponde à « sous quelle lecture émettre ? ».
///
/// Tant que les trois cadres portaient le même numéro, un producteur de dossier pouvait choisir son
/// gabarit à la main sans risque : `*_2026_06` se lisait d'un coup d'œil, et trois appels alignés se
/// vérifiaient à l'œil nu. Depuis que **les versions ne marchent plus au pas** ([`gdpr_2026_08`]), cette
/// vérification-là ne marche plus — et un producteur oublié continuerait d'émettre des dossiers **neufs**
/// sous une lecture **périmée**, en silence, en taisant à un régulateur un mécanisme réellement servi.
///
/// Ce point unique existe pour que la prochaine montée de version n'en laisse aucun derrière. Il y a deux
/// producteurs aujourd'hui (la passerelle d'essaim et la console d'audit) ; le prochain n'aura pas à
/// deviner.
pub fn courant(framework: Framework) -> MappingSet {
    match framework {
        Framework::EuAiAct => eu_ai_act_2026_06(),
        Framework::Gdpr => gdpr_2026_08(),
        // La CSDDD européenne et la LkSG allemande posent les mêmes exigences de traçabilité
        // fournisseur : un seul registre porte les deux, donc une seule lecture.
        Framework::Csddd | Framework::Lksg => csddd_lksg_2026_06(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_templates_construct_and_are_honest() {
        // I-S6-COUVERTURE-HONNÊTE au niveau gabarit : chacun construit (donc chaque cadre a un trou), et
        // déclare effectivement ≥ 1 hors-périmètre par cadre présent.
        for set in all_2026_06().into_iter().chain(all_courants()) {
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
        // prétendre que l'audit scellé est « compatible effacement » tel quel. Vrai des DEUX lectures :
        // la v1 le nommait comme une voie, la v2 le nomme comme un mécanisme — aucune ne l'escamote.
        for set in [gdpr_2026_06(), gdpr_2026_08()] {
            let hole = set.for_framework(Framework::Gdpr).into_iter().find(|m| m.coverage == Coverage::OutOfDagScope).unwrap();
            assert!(hole.does_not_furnish.contains("crypto-shredding"), "le crypto-shredding est nommé (v{})", set.version);
            assert!(hole.does_not_furnish.contains("art. 17"), "l'articulation avec l'art. 17 est nommée (v{})", set.version);
        }
    }

    /// **Le garde du gel.** Un dossier déjà émis sous « 2026-06 » ne se re-dérive à l'identique que si NI
    /// la v1 NI le socle qu'elle partage avec la v2 ne bougent — une propriété qu'aucun commentaire ne
    /// peut faire respecter. Cette constante la rend exécutable : toucher l'un ou l'autre casse le build.
    ///
    /// Si ce test tombe, la question n'est **jamais** « quel est le nouveau hash ? » mais « pourquoi une
    /// lecture figée a-t-elle changé ? ». La réponse correcte est presque toujours d'ouvrir une v3.
    #[test]
    fn the_frozen_gdpr_v1_is_pinned_by_hash() {
        let v1 = gdpr_2026_06();
        assert_eq!(v1.version, "2026-06");
        assert_eq!(v1.published_at, "2026-06-29");
        assert_eq!(
            v1.set_hash(),
            "f578b1fb7fd59c64007c309cb28f67bfde2c49a120405c3988025a4d71808a60",
            "la lecture de juin a changé de contenu — un dossier déjà émis sous « 2026-06 » ne se \
             re-dérive donc plus à l'identique. Ouvrez une nouvelle version au lieu de retoucher celle-ci."
        );
    }

    #[test]
    fn new_gdpr_content_travels_under_a_new_number() {
        // I-S6-MAPPING-VERSIONNÉ, dans le sens qui compte pour un régulateur. `set_hash` bouge TOUT SEUL
        // dès qu'on touche au contenu ; `version` est un littéral qui, lui, ne bouge que si on y pense.
        // C'est exactement là que naît le mensonge de version — et voici le test qui l'attrape.
        let (v1, v2) = (gdpr_2026_06(), gdpr_2026_08());
        assert_ne!(v1.set_hash(), v2.set_hash(), "le contenu a bien changé");
        assert_ne!(v1.version, v2.version, "…donc le NUMÉRO devait changer aussi — sinon deux lectures différentes voyagent sous le même nom");
        // Et l'inverse : les cadres dont la lecture n'a PAS changé gardent leur numéro. Re-estampiller
        // une lecture inchangée annoncerait un changement qui n'a pas eu lieu.
        assert_eq!(eu_ai_act_2026_06().version, "2026-06", "l'AI Act n'avait rien à re-déclarer");
        assert_eq!(csddd_lksg_2026_06().version, "2026-06", "le devoir de vigilance non plus");
    }

    #[test]
    fn gdpr_v2_tools_storage_limitation_without_promising_the_policy() {
        let v2 = gdpr_2026_08();
        let e = v2.for_framework(Framework::Gdpr).into_iter().find(|m| m.citation == "art. 5(1)(e)").expect("l'art. 5(1)(e) est mappé");
        // PARTIEL, jamais COUVERT : M6 fournit le mécanisme d'exécution, pas la politique de conservation.
        assert_eq!(e.coverage, Coverage::Partial, "le mécanisme n'est pas la politique");
        assert!(e.does_not_furnish.contains("AUCUNE politique"), "la politique reste au déployeur, dit en toutes lettres");
        // Les trois aveux que `shred.rs` exige de dire doivent VOYAGER dans l'artefact, pas rester dans
        // notre documentation : un régulateur lit le fichier rendu, pas nos commentaires.
        for aveu in ["OPT-IN", "rétroactif", "preuve de destruction", "Le nœud RESTE"] {
            assert!(e.does_not_furnish.contains(aveu), "l'aveu « {aveu} » doit voyager avec le dossier");
        }
        // Et c'est bien ce que la v2 AJOUTE — la v1 ne connaît pas cette ligne.
        assert!(gdpr_2026_06().for_framework(Framework::Gdpr).iter().all(|m| m.citation != "art. 5(1)(e)"), "l'art. 5(1)(e) est le delta de la v2");
    }

    #[test]
    fn every_framework_has_a_current_reading_from_one_place() {
        // Le défaut que ce point unique existe pour empêcher : un cadre dont la lecture courante est
        // décidée ailleurs, et qui reste en arrière à la montée de version suivante.
        assert_eq!(courant(Framework::Gdpr).version, VERSION_GDPR_V2, "le GDPR est passé en v2");
        assert_eq!(courant(Framework::EuAiAct).version, VERSION);
        // Les deux devoirs de vigilance partagent une lecture — ils doivent donc partager sa version.
        assert_eq!(courant(Framework::Csddd).version, courant(Framework::Lksg).version);
        // Et chaque cadre demandé est bien PRÉSENT dans la lecture qu'on rend pour lui : rendre un
        // registre qui ne parle pas du cadre demandé produirait un dossier vide plutôt qu'une erreur.
        for fw in [Framework::EuAiAct, Framework::Gdpr, Framework::Csddd, Framework::Lksg] {
            assert!(!courant(fw).for_framework(fw).is_empty(), "{} n'est pas mappé par sa propre lecture", fw.label());
        }
    }

    #[test]
    fn no_reading_ever_utters_a_verdict_of_conformity() {
        // La ligne rouge du produit, tenue à la SOURCE. `m5-check.sh` et le test de la passerelle la
        // vérifient sur le fichier rendu ; mais la prose qui pourrait la franchir s'écrit ICI, et un
        // gabarit n'est pas toujours rendu avant d'être lu. On la garde donc aux deux bouts.
        for set in all_2026_06().into_iter().chain(all_courants()) {
            for m in &set.mappings {
                let t = format!("{} {} {}", m.requirement_text, m.furnishes, m.does_not_furnish).to_lowercase();
                assert!(!t.contains("compliant"), "{} {} prononce un verdict de conformité", m.framework.label(), m.citation);
            }
        }
    }

    #[test]
    fn template_set_hash_is_stable_across_calls() {
        // Deux constructions du même gabarit → même hash (les gabarits sont des constantes, pas une horloge).
        assert_eq!(eu_ai_act_2026_06().set_hash(), eu_ai_act_2026_06().set_hash());
        // Les trois gabarits ont des hash distincts (contenus distincts).
        assert_eq!(gdpr_2026_08().set_hash(), gdpr_2026_08().set_hash(), "la v2 aussi est une constante");
        let hs: Vec<String> = all_2026_06().iter().map(|s| s.set_hash()).collect();
        let mut uniq = hs.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), hs.len(), "trois gabarits distincts → trois hash distincts");
    }
}
